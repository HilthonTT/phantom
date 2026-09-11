use std::{convert::AsRef, fmt::Debug, sync::Arc};

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement};
use serde::{Deserialize, Serialize};

use crate::{
    cursor::{FORWARD, REVERSE},
    keyval::{KeyVal, result_deserialize, serialize_key},
};

#[implement(super::Map)]
pub fn stream<'a, K, V>(
    self: &'a Arc<Self>,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
{
    self.raw_stream().map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
pub fn rev_stream<'a, K, V>(
    self: &'a Arc<Self>,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
{
    self.rev_raw_stream().map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_stream(self: &Arc<Self>) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_> {
    self.iter_from::<KeyVal<'_>, FORWARD>(None)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_stream(self: &Arc<Self>) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_> {
    self.iter_from::<KeyVal<'_>, REVERSE>(None)
}

#[implement(super::Map)]
pub fn stream_from<'a, K, V, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V, P>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let from = serialize_key(from).expect("failed to serialize query key");

    self.raw_stream_from(&from).map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
pub fn rev_stream_from<'a, K, V, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V, P>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let from = serialize_key(from).expect("failed to serialize query key");

    self.rev_raw_stream_from(&from)
        .map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn raw_stream_from<P>(
    self: &Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_, P>
where
    P: AsRef<[u8]> + ?Sized + Debug,
{
    self.iter_from::<KeyVal<'_>, FORWARD>(Some(from.as_ref()))
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn rev_raw_stream_from<P>(
    self: &Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_, P>
where
    P: AsRef<[u8]> + ?Sized + Debug,
{
    self.iter_from::<KeyVal<'_>, REVERSE>(Some(from.as_ref()))
}

#[implement(super::Map)]
pub fn stream_prefix<'a, K, V, P>(
    self: &'a Arc<Self>,
    prefix: &P,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V, P>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize query key");

    self.iter_prefix::<KeyVal<'a>, _, FORWARD>(prefix)
        .map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
pub fn rev_stream_prefix<'a, K, V, P>(
    self: &'a Arc<Self>,
    prefix: &P,
) -> impl Stream<Item = Result<KeyVal<'a, K, V>>> + Send + use<'a, K, V, P>
where
    K: Deserialize<'a> + Send,
    V: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize query key");

    self.iter_prefix::<KeyVal<'a>, _, REVERSE>(prefix)
        .map(result_deserialize::<K, V>)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, prefix), fields(%self), level = "trace")]
pub fn raw_stream_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &'a P,
) -> impl Stream<Item = Result<KeyVal<'a>>> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
    self.iter_prefix::<KeyVal<'a>, _, FORWARD>(prefix)
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, prefix), fields(%self), level = "trace")]
pub fn rev_raw_stream_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &'a P,
) -> impl Stream<Item = Result<KeyVal<'a>>> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
    self.iter_prefix::<KeyVal<'a>, _, REVERSE>(prefix)
}
