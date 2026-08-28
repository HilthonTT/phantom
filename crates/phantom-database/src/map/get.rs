//! Reading values by key, as bytes the caller already has.
//!
//! Keys that have to be built from a typed value go through
//! [`qry`](super::qry) and [`qry_batch`](super::qry_batch), which serialize
//! and then come back here.
//!
//! Every read here tries the block cache on the calling thread first. A hit
//! costs a lock and a memcmp; a miss comes back as `Incomplete` rather than as
//! an error, and is re-issued on [`the pool`](crate::pool), where blocking
//! until the storage answers is allowed.

use std::{convert::AsRef, fmt::Debug, sync::Arc};

use futures::{Future, FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt, future::ready};
use phantom_core::{
    Err, Result, err, implement,
    result::MapExpect,
    stream::{IterStream, WidebandExt, automatic_amplification, automatic_width},
};
use rocksdb::{DBPinnableSlice, ReadOptions};
use tokio::task;

use crate::{
    Handle,
    engine::error::{is_incomplete, map_err, or_else},
    pool,
};

/// Reads the value at `key`, which is used as-is.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn get<K>(
    self: &Arc<Self>,
    key: &K,
) -> impl Future<Output = Result<Handle<'_>>> + Send + use<'_, K>
where
    K: AsRef<[u8]> + Debug + ?Sized,
{
    let cached = self.get_cached(key);
    if matches!(cached, Err(_) | Ok(Some(_))) {
        // Answered without I/O, so nothing here will await. Yielding the
        // cooperative budget is what keeps a long run of cache hits from
        // monopolising the tokio worker it is running on.
        return task::consume_budget()
            .map(move |()| cached.map_expect("a cached read was already resolved"))
            .boxed();
    }

    debug_assert!(matches!(cached, Ok(None)), "expected an incomplete read");

    self.db
        .pool
        .execute_get(pool::Get {
            map: self.clone(),
            key: [key.as_ref().into()].into(),
            res: None,
        })
        .and_then(|mut res| ready(res.remove(0)))
        .boxed()
}

/// Reads the values at each of `keys`, in order.
///
/// Keys are gathered into batches so that one submission to the pool covers
/// many of them, and the batches run concurrently. Both figures come from
/// [`the stream tuning`](phantom_core::stream), which the pool sets
/// from the storage topology at startup.
#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), level = "trace")]
pub fn get_batch<'a, S, K>(
    self: &'a Arc<Self>,
    keys: S,
) -> impl Stream<Item = Result<Handle<'a>>> + Send + 'a
where
    S: Stream<Item = K> + Send + 'a,
    K: AsRef<[u8]> + Send + Sync + 'a,
{
    keys.ready_chunks(automatic_amplification())
        .widen_then(automatic_width(), |chunk| {
            self.db.pool.execute_get(pool::Get {
                map: self.clone(),
                key: chunk.iter().map(AsRef::as_ref).map(Into::into).collect(),
                res: None,
            })
        })
        .map_ok(|results| results.into_iter().stream())
        .try_flatten()
}

/// Reads the value at `key`, blocking until the storage answers.
///
/// For callers already on a thread where blocking is allowed — a pool worker,
/// or an operator command running on its own thread. Everything reached from a
/// tokio worker should use [`Self::get`].
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), name = "blocking", level = "trace")]
pub fn get_blocking<K>(&self, key: &K) -> Result<Handle<'_>>
where
    K: AsRef<[u8]> + ?Sized,
{
    handle_from(self.get_blocking_opts(key, &self.read_options))
}

/// [`Self::get_blocking`] over many keys at once, which lets the engine sort
/// the reads and coalesce those landing in the same block.
#[implement(super::Map)]
#[tracing::instrument(name = "batch_blocking", level = "trace", skip_all)]
pub(crate) fn get_batch_blocking<'a, 'b, I, K>(
    &'b self,
    keys: I,
) -> impl Iterator<Item = Result<Handle<'b>>> + Send + use<'a, 'b, I, K>
where
    I: Iterator<Item = &'a K> + ExactSizeIterator + Send,
    K: AsRef<[u8]> + Send + ?Sized + Sync + 'a,
{
    self.get_batch_blocking_opts(keys, &self.read_options)
        .map(handle_from)
}

/// Reads the value at `key` if the block cache already holds it.
///
/// `Ok(None)` is a cache miss — the value may well exist on disk — whereas a
/// `NotFound` error is the cache answering that it does not.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), name = "cached", level = "trace")]
pub(crate) fn get_cached<K>(&self, key: &K) -> Result<Option<Handle<'_>>>
where
    K: AsRef<[u8]> + Debug + ?Sized,
{
    cached_handle_from(self.get_blocking_opts(key, &self.cache_read_options))
}

#[implement(super::Map)]
fn get_blocking_opts<K>(
    &self,
    key: &K,
    read_options: &ReadOptions,
) -> Result<Option<DBPinnableSlice<'_>>, rocksdb::Error>
where
    K: AsRef<[u8]> + ?Sized,
{
    self.db.db.get_pinned_cf_opt(&self.cf(), key, read_options)
}

#[implement(super::Map)]
fn get_batch_blocking_opts<'a, 'b, I, K>(
    &'b self,
    keys: I,
    read_options: &ReadOptions,
) -> impl Iterator<Item = Result<Option<DBPinnableSlice<'b>>, rocksdb::Error>> + Send + use<'a, 'b, I, K>
where
    I: Iterator<Item = &'a K> + ExactSizeIterator + Send,
    K: AsRef<[u8]> + Send + ?Sized + Sync + 'a,
{
    /// The engine can skip its own sort where the keys already arrive in the
    /// column's order. Callers do not promise that, so it does the sort.
    const SORTED: bool = false;

    self.db
        .db
        .batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
        .into_iter()
}

/// A read that was allowed to go to disk: absent means absent.
#[inline]
pub(super) fn handle_from(
    result: Result<Option<DBPinnableSlice<'_>>, rocksdb::Error>,
) -> Result<Handle<'_>> {
    result
        .map_err(map_err)?
        .map(Handle::from)
        .ok_or_else(|| err!(Request(NotFound("Not found in database"))))
}

/// A cache-only read, where absent and unknown are different answers.
#[inline]
pub(super) fn cached_handle_from(
    result: Result<Option<DBPinnableSlice<'_>>, rocksdb::Error>,
) -> Result<Option<Handle<'_>>> {
    match result {
        // Hit, and the key is not there.
        Ok(None) => Err!(Request(NotFound("Not found in database"))),

        // Hit.
        Ok(Some(result)) => Ok(Some(Handle::from(result))),

        // Miss: the cache declined to answer, so the caller must go to disk.
        Err(ref error) if is_incomplete(error) => Ok(None),

        Err(error) => or_else(error),
    }
}
