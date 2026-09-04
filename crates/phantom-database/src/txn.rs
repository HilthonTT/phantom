//! Writes across several columns that land together or not at all.
//!
//! [`Map::insert_batch`](crate::Map) already batches, but only within one
//! column. A great deal of what this server writes is a pair: an index entry
//! and the thing it indexes, a value moved from one column to another. Written
//! separately, a crash between them leaves the two disagreeing — an event
//! marked in flight that is no longer queued, a room whose alias resolves to
//! nothing.
//!
//! A [`Txn`] is one RocksDB write batch over the columns of one engine. Nothing
//! is written until [`Txn::execute`] is called, and what it writes is applied
//! atomically.
//!
//! Unlike the reference, the keys written are recorded as they are queued
//! rather than recovered afterwards by decoding the batch's own representation.
//! That decoding is what the watchers need in order to be woken, and it is a
//! parser for a private RocksDB encoding — record tags and varints — that has
//! no reason to stay stable. Keeping the keys costs an allocation per entry.

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

/// A set of writes over one engine's columns, applied together.
///
/// Build one with [`Txn::new`], queue writes onto it, and call
/// [`Txn::execute`]. Dropping one instead leaves the database untouched, which
/// is what makes an error part-way through building one safe to bail out of.
#[must_use = "a transaction does nothing until execute() is called"]
pub struct Txn {
    batch: WriteBatchWithTransaction<false>,
    engine: Arc<Engine>,

    /// The columns and keys written, so the watchers can be woken once the
    /// batch has actually landed.
    written: Vec<(Arc<Map>, Vec<u8>)>,
}

impl Txn {
    /// An empty transaction over `engine`'s columns.
    pub fn new(engine: &Arc<Engine>) -> Self {
        Self {
            batch: WriteBatchWithTransaction::default(),
            engine: engine.clone(),
            written: Vec::new(),
        }
    }

    /// How many operations are queued.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.batch.len()
    }

    /// Whether nothing is queued, in which case executing writes nothing.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }
}

/// Queues a write of `val` at `key`, serializing both.
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

/// [`put`](Self::put) with an already-serialized value.
#[implement(Txn)]
pub fn put_raw<K, V>(&mut self, map: &Arc<Map>, key: K, val: V) -> Result<&mut Self>
where
    K: Serialize + Debug,
    V: AsRef<[u8]>,
{
    let key = serialize_key(key)?;

    Ok(self.insert(map, key, val))
}

/// Queues a write of `val` at `key`, both used as-is.
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

/// Queues a delete at `key`, serializing it. Deleting an absent key is not an
/// error, here or on execution.
#[implement(Txn)]
pub fn del<K>(&mut self, map: &Arc<Map>, key: K) -> Result<&mut Self>
where
    K: Serialize + Debug,
{
    let key = serialize_key(key)?;

    Ok(self.remove(map, key))
}

/// Queues a delete at `key`, used as-is.
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

    // A delete is a change under the key as much as a write is, so a watcher
    // parked on the prefix wants waking for it too.
    self.written.push((map.clone(), key.as_ref().to_vec()));

    self
}

/// Applies every queued operation, atomically.
///
/// The whole batch lands or none of it does. The watchers are woken afterwards,
/// so nothing is told about a write that then failed.
///
/// The log flush is the same one an ordinary write does, and is held back the
/// same way by a [`Cork`](crate::Cork).
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

/// Whether `map` belongs to this transaction's engine.
///
/// A column family handle only means anything to the database it came from, so
/// writing one map's column through another database's batch would write
/// somewhere unrelated.
#[implement(Txn)]
fn owns(&self, map: &Arc<Map>) -> bool {
    Arc::ptr_eq(&self.engine, map.db())
}
