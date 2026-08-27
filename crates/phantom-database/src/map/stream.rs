//! Iterating a column's entries, keys and values together.
//!
//! The same shapes as [`keys`](super::keys), yielding
//! [`KeyVal`](crate::keyval::KeyVal) instead of a key alone. Where only the
//! keys are wanted, iterate there instead: the values live in blocks this has
//! to read and that does not.
//!
//! Every method has a reverse twin under the same name with a `rev_` prefix.

use std::{convert::AsRef, fmt::Debug, sync::Arc};

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement};
use serde::{Deserialize, Serialize};

use crate::{
    cursor::{FORWARD, REVERSE},
    keyval::{KeyVal, result_deserialize, serialize_key},
};

/// Every entry in the column, in key order.
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

/// [`Self::stream`] from the end of the column backwards.
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

/// Every entry in the column, as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_stream(self: &Arc<Self>) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_> {
    self.iter_from::<KeyVal<'_>, FORWARD>(None)
}

/// [`Self::raw_stream`] from the end of the column backwards.
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_stream(self: &Arc<Self>) -> impl Stream<Item = Result<KeyVal<'_>>> + Send + use<'_> {
    self.iter_from::<KeyVal<'_>, REVERSE>(None)
}

/// Every entry whose key is at or after `from`.
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

/// Every entry whose key is at or before `from`, backwards.
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

/// [`Self::stream_from`] with the bound and the results as bytes.
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

/// [`Self::rev_stream_from`] with the bound and the results as bytes.
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

/// Every entry whose key begins with `prefix`.
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

/// [`Self::stream_prefix`] from the end of the prefix's range backwards.
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

/// [`Self::stream_prefix`] with the prefix and the results as bytes.
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

/// [`Self::rev_stream_prefix`] with the prefix and the results as bytes.
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
