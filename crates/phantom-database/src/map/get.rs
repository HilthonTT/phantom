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

#[implement(super::Map)]
#[tracing::instrument(skip(self, key), name = "blocking", level = "trace")]
pub fn get_blocking<K>(&self, key: &K) -> Result<Handle<'_>>
where
    K: AsRef<[u8]> + ?Sized,
{
    handle_from(self.get_blocking_opts(key, &self.read_options))
}

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
    const SORTED: bool = false;

    self.db
        .db
        .batched_multi_get_cf_opt(&self.cf(), keys, SORTED, read_options)
        .into_iter()
}

#[inline]
pub(super) fn handle_from(
    result: Result<Option<DBPinnableSlice<'_>>, rocksdb::Error>,
) -> Result<Handle<'_>> {
    result
        .map_err(map_err)?
        .map(Handle::from)
        .ok_or_else(|| err!(Request(NotFound("Not found in database"))))
}

#[inline]
pub(super) fn cached_handle_from(
    result: Result<Option<DBPinnableSlice<'_>>, rocksdb::Error>,
) -> Result<Option<Handle<'_>>> {
    match result {
        Ok(None) => Err!(Request(NotFound("Not found in database"))),

        Ok(Some(result)) => Ok(Some(Handle::from(result))),

        Err(ref error) if is_incomplete(error) => Ok(None),

        Err(error) => or_else(error),
    }
}
