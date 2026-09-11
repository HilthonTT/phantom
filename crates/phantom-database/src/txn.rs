use std::{fmt::Debug, sync::Arc};

use phantom_core::{Result, implement};
use rocksdb::WriteBatchWithTransaction;
use serde::Serialize;

use crate::{
    Engine, Map,
    engine::error::result,
    keyval::{serialize_key, serialize_val},
    map::write_options_default,
};

#[must_use = "a transaction does nothing until execute() is called"]
pub struct Txn {
    batch: WriteBatchWithTransaction<false>,
    engine: Arc<Engine>,

    written: Vec<(Arc<Map>, Vec<u8>)>,
}

impl Txn {
    pub fn new(engine: &Arc<Engine>) -> Self {
        Self {
            batch: WriteBatchWithTransaction::default(),
            engine: engine.clone(),
            written: Vec::new(),
        }
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }
}

#[implement(Txn)]
pub fn put<K, V>(&mut self, map: &Arc<Map>, key: K, val: V) -> Result<&mut Self>
where
    K: Serialize + Debug,
    V: Serialize,
{
    let key = serialize_key(key)?;
    let val = serialize_val(val)?;

    Ok(self.insert(map, key, val))
}

#[implement(Txn)]
pub fn put_raw<K, V>(&mut self, map: &Arc<Map>, key: K, val: V) -> Result<&mut Self>
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
{
    let key = serialize_key(key)?;

    Ok(self.insert(map, key, val))
}

#[implement(Txn)]
pub fn insert<K, V>(&mut self, map: &Arc<Map>, key: K, val: V) -> &mut Self
where
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
{
    debug_assert!(
        self.owns(map),
        "a transaction may only write to the columns of the engine it was opened on"
    );

    self.batch.put_cf(&map.cf(), key.as_ref(), val.as_ref());
    self.written.push((map.clone(), key.as_ref().to_vec()));

    self
}

#[implement(Txn)]
pub fn del<K>(&mut self, map: &Arc<Map>, key: K) -> Result<&mut Self>
where
    K: Serialize + Debug,
{
    let key = serialize_key(key)?;

    Ok(self.remove(map, key))
}

#[implement(Txn)]
pub fn remove<K>(&mut self, map: &Arc<Map>, key: K) -> &mut Self
where
    K: AsRef<[u8]>,
{
    debug_assert!(
        self.owns(map),
        "a transaction may only write to the columns of the engine it was opened on"
    );

    self.batch.delete_cf(&map.cf(), key.as_ref());

    self.written.push((map.clone(), key.as_ref().to_vec()));

    self
}

#[implement(Txn)]
#[tracing::instrument(name = "txn", level = "trace", skip_all, fields(ops = %self.len()))]
pub fn execute(self) -> Result {
    let Self {
        batch,
        engine,
        written,
    } = self;

    if batch.is_empty() {
        return Ok(());
    }

    result(engine.db.write_opt(batch, &write_options_default(&engine)))?;

    if !engine.corked() {
        engine.flush()?;
    }

    for (map, key) in &written {
        map.wake(key);
    }

    Ok(())
}

#[implement(Txn)]
fn owns(&self, map: &Arc<Map>) -> bool {
    Arc::ptr_eq(&self.engine, map.db())
}
