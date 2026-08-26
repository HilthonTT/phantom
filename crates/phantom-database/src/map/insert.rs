//! Writing and deleting entries.
//!
//! Writes are synchronous: they go into the memtable and the write-ahead log
//! on the calling thread, which does not touch the storage. What does is the
//! log flush afterwards, and that is what a [`Cork`](crate::Cork) holds back —
//! without one, a run of writes costs a flush each.
//!
//! Unlike the reads, these return [`Result`] rather than panicking on a failed
//! write. A write that fails usually means the database is no longer usable,
//! and most callers will have nothing better to do than give up; making that
//! their decision rather than this layer's is what lets the ones that do have
//! something better to do — an operator command, a migration — take it.

use std::{convert::AsRef, fmt::Debug, io::Write};

use arrayvec::ArrayVec;
use phantom_core::{Result, implement};
use rocksdb::WriteBatchWithTransaction;
use serde::Serialize;

use crate::{
    keyval::{KeyBuf, ValBuf},
    ser,
    util::result,
};

/// Writes `val` at `key`, serializing both.
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

/// [`Self::put`] with an already-serialized value.
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

/// [`Self::put`] with an already-serialized key.
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

/// [`Self::put`], serializing both halves into stack buffers. See
/// [`Map::aqry`](super::Map::aqry) for when the fixed sizes are appropriate.
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

/// [`Self::aput`] with an already-serialized value.
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

/// [`Self::aput`] with an already-serialized key.
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

/// [`Self::put`], serializing into buffers the caller supplies.
#[implement(super::Map)]
pub fn bput<K, V, Bk, Bv>(&self, key: K, val: V, mut buf: (Bk, Bv)) -> Result
where
    K: Serialize + Debug,
    V: Serialize,
    Bk: Write + AsRef<[u8]>,
    Bv: Write + AsRef<[u8]>,
{
    let val = ser::serialize(&mut buf.1, val)?;

    self.bput_raw(key, val, &mut buf.0)
}

/// [`Self::bput`] with an already-serialized value.
#[implement(super::Map)]
#[tracing::instrument(skip(self, val, buf), level = "trace")]
pub fn bput_raw<K, V, Bk>(&self, key: K, val: V, mut buf: Bk) -> Result
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
    Bk: Write + AsRef<[u8]>,
{
    let key = ser::serialize(&mut buf, key)?;

    self.insert(&key, val)
}

/// [`Self::bput`] with an already-serialized key.
#[implement(super::Map)]
pub fn raw_bput<K, V, Bv>(&self, key: K, val: V, mut buf: Bv) -> Result
where
    K: AsRef<[u8]>,
    V: Serialize,
    Bv: Write + AsRef<[u8]>,
{
    let val = ser::serialize(&mut buf, val)?;

    self.insert(&key, val)
}

/// Writes `val` at `key`, both used as-is.
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

/// Writes every entry of `iter` as one batch.
///
/// The batch is atomic and costs a single log flush, which is what makes this
/// worth reaching for over a run of [`Self::insert`] — but it is held in
/// memory until it is written, so it is for a bounded run rather than an
/// unbounded stream.
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

    // Only once the batch is durable: a waiter woken by a write that then
    // failed would read state that never existed.
    for key in &keys {
        self.watchers.wake(key.as_ref());
    }

    Ok(())
}

/// Deletes the entry at a key built from `key`. Deleting an absent key is not
/// an error.
#[implement(super::Map)]
#[inline]
pub fn del<K>(&self, key: K) -> Result
where
    K: Serialize + Debug,
{
    let mut buf = KeyBuf::new();

    self.bdel(key, &mut buf)
}

/// [`Self::del`], serializing into a stack buffer of `MAX` bytes.
#[implement(super::Map)]
#[inline]
pub fn adel<const MAX: usize, K>(&self, key: K) -> Result
where
    K: Serialize + Debug,
{
    let mut buf = ArrayVec::<u8, MAX>::new();

    self.bdel(key, &mut buf)
}

/// [`Self::del`], serializing into a buffer the caller supplies.
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), level = "trace")]
pub fn bdel<K, B>(&self, key: K, buf: &mut B) -> Result
where
    K: Serialize + Debug,
    B: Write + AsRef<[u8]>,
{
    let key = ser::serialize(buf, key)?;

    self.remove(key)
}

/// Deletes the entry at `key`, which is used as-is.
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

/// Flushes the write-ahead log unless a [`Cork`](crate::Cork) is holding it
/// back, in which case the cork's drop will do it.
#[implement(super::Map)]
#[inline]
fn flush_if_uncorked(&self) -> Result {
    if self.db.corked() {
        return Ok(());
    }

    self.db.flush()
}
