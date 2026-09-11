use std::{convert::AsRef, fmt::Debug, io::Write};

use arrayvec::ArrayVec;
use phantom_core::{Result, implement};
use rocksdb::WriteBatchWithTransaction;
use serde::Serialize;

use crate::{
    codec::serialize::serialize,
    engine::error::result,
    keyval::{KeyBuf, ValBuf},
};

#[implement(super::Map)]
#[inline]
pub fn put<K, V>(&self, key: K, val: V) -> Result
where
    K: Serialize + Debug,
    V: Serialize,
{
    let mut key_buf = KeyBuf::new();
    let mut val_buf = ValBuf::new();

    self.bput(key, val, (&mut key_buf, &mut val_buf))
}

#[implement(super::Map)]
#[inline]
pub fn put_raw<K, V>(&self, key: K, val: V) -> Result
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
{
    let mut key_buf = KeyBuf::new();

    self.bput_raw(key, val, &mut key_buf)
}

#[implement(super::Map)]
#[inline]
pub fn raw_put<K, V>(&self, key: K, val: V) -> Result
where
    K: AsRef<[u8]>,
    V: Serialize,
{
    let mut val_buf = ValBuf::new();

    self.raw_bput(key, val, &mut val_buf)
}

#[implement(super::Map)]
#[inline]
pub fn aput<const KMAX: usize, const VMAX: usize, K, V>(&self, key: K, val: V) -> Result
where
    K: Serialize + Debug,
    V: Serialize,
{
    let mut key_buf = ArrayVec::<u8, KMAX>::new();
    let mut val_buf = ArrayVec::<u8, VMAX>::new();

    self.bput(key, val, (&mut key_buf, &mut val_buf))
}

#[implement(super::Map)]
#[inline]
pub fn aput_raw<const KMAX: usize, K, V>(&self, key: K, val: V) -> Result
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
{
    let mut key_buf = ArrayVec::<u8, KMAX>::new();

    self.bput_raw(key, val, &mut key_buf)
}

#[implement(super::Map)]
#[inline]
pub fn raw_aput<const VMAX: usize, K, V>(&self, key: K, val: V) -> Result
where
    K: AsRef<[u8]>,
    V: Serialize,
{
    let mut val_buf = ArrayVec::<u8, VMAX>::new();

    self.raw_bput(key, val, &mut val_buf)
}

#[implement(super::Map)]
pub fn bput<K, V, Bk, Bv>(&self, key: K, val: V, mut buf: (Bk, Bv)) -> Result
where
    K: Serialize + Debug,
    V: Serialize,
    Bk: Write + AsRef<[u8]>,
    Bv: Write + AsRef<[u8]>,
{
    let val = serialize(&mut buf.1, val)?;

    self.bput_raw(key, val, &mut buf.0)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, val, buf), level = "trace")]
pub fn bput_raw<K, V, Bk>(&self, key: K, val: V, mut buf: Bk) -> Result
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
    Bk: Write + AsRef<[u8]>,
{
    let key = serialize(&mut buf, key)?;

    self.insert(&key, val)
}

#[implement(super::Map)]
pub fn raw_bput<K, V, Bv>(&self, key: K, val: V, mut buf: Bv) -> Result
where
    K: AsRef<[u8]>,
    V: Serialize,
    Bv: Write + AsRef<[u8]>,
{
    let val = serialize(&mut buf, val)?;

    self.insert(&key, val)
}

#[implement(super::Map)]
#[tracing::instrument(skip_all, fields(%self), level = "trace")]
pub fn insert<K, V>(&self, key: &K, val: V) -> Result
where
    K: AsRef<[u8]> + ?Sized,
    V: AsRef<[u8]>,
{
    result(
        self.db
            .db
            .put_cf_opt(&self.cf(), key, val, &self.write_options),
    )?;

    self.flush_if_uncorked()?;
    self.watchers.wake(key.as_ref());

    Ok(())
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, iter), fields(%self), level = "trace")]
pub fn insert_batch<'a, I, K, V>(&'a self, iter: I) -> Result
where
    I: Iterator<Item = (K, V)> + Send,
    K: AsRef<[u8]> + Sized + 'a,
    V: AsRef<[u8]> + Sized + 'a,
{
    let mut batch = WriteBatchWithTransaction::<false>::default();
    let mut keys = Vec::new();

    for (key, val) in iter {
        batch.put_cf(&self.cf(), key.as_ref(), val.as_ref());
        keys.push(key);
    }

    result(self.db.db.write_opt(batch, &self.write_options))?;
    self.flush_if_uncorked()?;

    for key in &keys {
        self.watchers.wake(key.as_ref());
    }

    Ok(())
}

#[implement(super::Map)]
#[inline]
pub fn del<K>(&self, key: K) -> Result
where
    K: Serialize + Debug,
{
    let mut buf = KeyBuf::new();

    self.bdel(key, &mut buf)
}

#[implement(super::Map)]
#[inline]
pub fn adel<const MAX: usize, K>(&self, key: K) -> Result
where
    K: Serialize + Debug,
{
    let mut buf = ArrayVec::<u8, MAX>::new();

    self.bdel(key, &mut buf)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), level = "trace")]
pub fn bdel<K, B>(&self, key: K, buf: &mut B) -> Result
where
    K: Serialize + Debug,
    B: Write + AsRef<[u8]>,
{
    let key = serialize(buf, key)?;

    self.remove(key)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn remove<K>(&self, key: &K) -> Result
where
    K: AsRef<[u8]> + ?Sized,
{
    result(
        self.db
            .db
            .delete_cf_opt(&self.cf(), key, &self.write_options),
    )?;

    self.flush_if_uncorked()
}

#[implement(super::Map)]
#[inline]
fn flush_if_uncorked(&self) -> Result {
    if self.db.corked() {
        return Ok(());
    }

    self.db.flush()
}
