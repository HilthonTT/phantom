//! Iterating a column's keys, without reading its values.
//!
//! The engine stores keys in the index blocks and values in the data blocks,
//! so an iteration that never looks at a value can leave the data blocks on
//! disk. Anything that only needs to know which keys exist — a count, a
//! membership scan, the key half of a secondary index — should come through
//! here rather than through [`stream`](super::stream) with the value dropped.
//!
//! Every method has a reverse twin under the same name with a `rev_` prefix,
//! yielding the same keys from the other end of the column.

use std::{convert::AsRef, fmt::Debug, sync::Arc};

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement};
use serde::{Deserialize, Serialize};

use crate::{
    keyval::{Key, result_deserialize_key, serialize_key},
    stream::{FORWARD, REVERSE},
};

/// Every key in the column, in order.
#[implement(super::Map)]
pub fn keys<'a, K>(
    self: &'a Arc<Self>,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K>
where
    K: Deserialize<'a> + Send,
{
    self.raw_keys().map(result_deserialize_key::<K>)
}

/// [`Self::keys`] from the end of the column backwards.
#[implement(super::Map)]
pub fn rev_keys<'a, K>(
    self: &'a Arc<Self>,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K>
where
    K: Deserialize<'a> + Send,
{
    self.rev_raw_keys().map(result_deserialize_key::<K>)
}

/// Every key in the column, as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn raw_keys(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_> {
    self.iter_from::<Key<'_>, FORWARD>(None)
}

/// [`Self::raw_keys`] from the end of the column backwards.
#[implement(super::Map)]
#[tracing::instrument(skip(self), fields(%self), level = "trace")]
pub fn rev_raw_keys(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_> {
    self.iter_from::<Key<'_>, REVERSE>(None)
}

/// Every key at or after `from`.
#[implement(super::Map)]
pub fn keys_from<'a, K, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K, P>
where
    K: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let from = serialize_key(from).expect("failed to serialize query key");

    self.raw_keys_from(&from).map(result_deserialize_key::<K>)
}

/// Every key at or before `from`, backwards.
#[implement(super::Map)]
pub fn rev_keys_from<'a, K, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K, P>
where
    K: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let from = serialize_key(from).expect("failed to serialize query key");

    self.rev_raw_keys_from(&from)
        .map(result_deserialize_key::<K>)
}

/// [`Self::keys_from`] with the bound and the results as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn raw_keys_from<P>(
    self: &Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_, P>
where
    P: AsRef<[u8]> + ?Sized + Debug,
{
    self.iter_from::<Key<'_>, FORWARD>(Some(from.as_ref()))
}

/// [`Self::rev_keys_from`] with the bound and the results as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self, from), fields(%self), level = "trace")]
pub fn rev_raw_keys_from<P>(
    self: &Arc<Self>,
    from: &P,
) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_, P>
where
    P: AsRef<[u8]> + ?Sized + Debug,
{
    self.iter_from::<Key<'_>, REVERSE>(Some(from.as_ref()))
}

/// Every key beginning with `prefix`.
#[implement(super::Map)]
pub fn keys_prefix<'a, K, P>(
    self: &'a Arc<Self>,
    prefix: &P,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K, P>
where
    K: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize query key");

    self.iter_prefix::<Key<'a>, _, FORWARD>(prefix)
        .map(result_deserialize_key::<K>)
}

/// [`Self::keys_prefix`] from the end of the prefix's range backwards.
#[implement(super::Map)]
pub fn rev_keys_prefix<'a, K, P>(
    self: &'a Arc<Self>,
    prefix: &P,
) -> impl Stream<Item = Result<Key<'a, K>>> + Send + use<'a, K, P>
where
    K: Deserialize<'a> + Send,
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize query key");

    self.iter_prefix::<Key<'a>, _, REVERSE>(prefix)
        .map(result_deserialize_key::<K>)
}

/// [`Self::keys_prefix`] with the prefix and the results as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self, prefix), fields(%self), level = "trace")]
pub fn raw_keys_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &'a P,
) -> impl Stream<Item = Result<Key<'a>>> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
    self.iter_prefix::<Key<'a>, _, FORWARD>(prefix)
}

/// [`Self::rev_keys_prefix`] with the prefix and the results as bytes.
#[implement(super::Map)]
#[tracing::instrument(skip(self, prefix), fields(%self), level = "trace")]
pub fn rev_raw_keys_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &'a P,
) -> impl Stream<Item = Result<Key<'a>>> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
    self.iter_prefix::<Key<'a>, _, REVERSE>(prefix)
}
