//! Reading values by a key that is serialized first.
//!
//! [`get`](super::get) takes the key as bytes it can hand straight to the
//! engine. Everything here builds those bytes from a typed key and then defers
//! to it, so the three forms differ only in where the serialized key is put:
//! a fresh buffer, a stack buffer of a size the caller names, or a buffer the
//! caller already holds.

use std::{convert::AsRef, fmt::Debug, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use phantom_core::{Result, implement};
use serde::Serialize;

use crate::{Handle, codec::serialize::serialize, keyval::KeyBuf};

/// Reads the value at a key built from `key`.
#[implement(super::Map)]
#[inline]
pub fn qry<K>(
    self: &Arc<Self>,
    key: &K,
) -> impl Future<Output = Result<Handle<'_>>> + Send + use<'_, K>
where
    K: Serialize + ?Sized + Debug,
{
    let mut buf = KeyBuf::new();

    self.bqry(key, &mut buf)
}

/// [`Self::qry`], serializing into a stack buffer of `MAX` bytes.
///
/// # Panics
///
/// If the serialized key does not fit. Use where the key's size is fixed by
/// its type — a pair of integers, say — and `MAX` can be read off it.
#[implement(super::Map)]
#[inline]
pub fn aqry<const MAX: usize, K>(
    self: &Arc<Self>,
    key: &K,
) -> impl Future<Output = Result<Handle<'_>>> + Send + use<'_, MAX, K>
where
    K: Serialize + ?Sized + Debug,
{
    let mut buf = ArrayVec::<u8, MAX>::new();

    self.bqry(key, &mut buf)
}

/// [`Self::qry`], serializing into a buffer the caller supplies, for a caller
/// reusing one across a run of queries.
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), level = "trace")]
pub fn bqry<K, B>(
    self: &Arc<Self>,
    key: &K,
    buf: &mut B,
) -> impl Future<Output = Result<Handle<'_>>> + Send + use<'_, K, B>
where
    K: Serialize + ?Sized + Debug,
    B: Write + AsRef<[u8]>,
{
    let key = serialize(buf, key).expect("failed to serialize query key");

    self.get(key)
}
