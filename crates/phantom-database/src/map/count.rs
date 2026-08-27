//! Counting entries.
//!
//! Every count here walks the keys, and only the keys: the values are in
//! blocks the iteration never has to read. The engine can also estimate a
//! column's size from its metadata without reading anything — see
//! [`Map::property_integer`](super::Map::property_integer) — but that is an
//! estimate, and it cannot answer for a range. These are exact.

use std::{convert::AsRef, fmt::Debug, future::Future, sync::Arc};

use futures::StreamExt;
use phantom_core::implement;
use serde::Serialize;

use crate::{
    cursor::FORWARD,
    keyval::{Key, serialize_key},
};

/// Entries in the column.
#[implement(super::Map)]
#[inline]
pub fn count(self: &Arc<Self>) -> impl Future<Output = usize> + Send + use<'_> {
    self.raw_keys().count()
}

/// Entries at or after `from`.
#[implement(super::Map)]
pub fn count_from<'a, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Future<Output = usize> + Send + use<'a, P>
where
    P: Serialize + ?Sized + Debug,
{
    let from = serialize_key(from).expect("failed to serialize query key");

    self.raw_keys_from(&from).count()
}

/// [`Self::count_from`] with the bound as bytes.
#[implement(super::Map)]
#[inline]
pub fn raw_count_from<'a, P>(
    self: &'a Arc<Self>,
    from: &P,
) -> impl Future<Output = usize> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug,
{
    self.raw_keys_from(from).count()
}

/// Entries whose keys begin with `prefix`.
#[implement(super::Map)]
pub fn count_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &P,
) -> impl Future<Output = usize> + Send + use<'a, P>
where
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize query key");

    self.iter_prefix::<Key<'a>, _, FORWARD>(prefix).count()
}

/// [`Self::count_prefix`] with the prefix as bytes.
#[implement(super::Map)]
#[inline]
pub fn raw_count_prefix<'a, P>(
    self: &'a Arc<Self>,
    prefix: &'a P,
) -> impl Future<Output = usize> + Send + use<'a, P>
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync + 'a,
{
    self.raw_keys_prefix(prefix).count()
}
