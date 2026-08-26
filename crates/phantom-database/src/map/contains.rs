//! Testing whether a key is present, without paying for its value.

use std::{convert::AsRef, fmt::Debug, future::Future, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use futures::FutureExt;
use phantom_core::{Result, err, implement, result::FlatOk, utils::future::TryExtExt};
use serde::Serialize;

use crate::{keyval::KeyBuf, ser};

/// Whether the column holds a key built from `key`.
#[implement(super::Map)]
#[inline]
pub fn contains<K>(self: &Arc<Self>, key: &K) -> impl Future<Output = bool> + Send + use<'_, K>
where
    K: Serialize + ?Sized + Debug,
{
    let mut buf = KeyBuf::new();

    self.bcontains(key, &mut buf)
}

/// [`Self::contains`], serializing into a stack buffer of `MAX` bytes. See
/// [`Map::aqry`](super::Map::aqry).
#[implement(super::Map)]
#[inline]
pub fn acontains<const MAX: usize, K>(
    self: &Arc<Self>,
    key: &K,
) -> impl Future<Output = bool> + Send + use<'_, MAX, K>
where
    K: Serialize + ?Sized + Debug,
{
    let mut buf = ArrayVec::<u8, MAX>::new();

    self.bcontains(key, &mut buf)
}

/// [`Self::contains`], serializing into a buffer the caller supplies.
#[implement(super::Map)]
#[tracing::instrument(skip(self, buf), fields(%self), level = "trace")]
pub fn bcontains<K, B>(
    self: &Arc<Self>,
    key: &K,
    buf: &mut B,
) -> impl Future<Output = bool> + Send + use<'_, K, B>
where
    K: Serialize + ?Sized + Debug,
    B: Write + AsRef<[u8]>,
{
    let key = ser::serialize(buf, key).expect("failed to serialize query key");

    self.exists(key).is_ok()
}

/// Whether the column holds `key`, which is used as-is.
///
/// Returns the failure where the read failed rather than found nothing, which
/// is the difference from [`Self::contains`].
#[implement(super::Map)]
#[inline]
pub fn exists<'a, K>(
    self: &'a Arc<Self>,
    key: &K,
) -> impl Future<Output = Result> + Send + use<'a, K>
where
    K: AsRef<[u8]> + ?Sized + Debug + 'a,
{
    self.get(key).map(|res| res.map(|_| ()))
}

/// [`Self::exists`], blocking until the storage answers.
///
/// Consults the engine's bloom filters first, which can rule the key out
/// without a read at all.
#[implement(super::Map)]
#[tracing::instrument(skip(self, key), fields(%self), level = "trace")]
pub fn exists_blocking<K>(&self, key: &K) -> Result
where
    K: AsRef<[u8]> + ?Sized + Debug,
{
    self.maybe_exists(key)
        .then(|| self.get_blocking(key))
        .flat_ok()
        .map(|_| ())
        .ok_or_else(|| err!(Request(NotFound("Not found in database"))))
}

/// Whether the key might be present.
///
/// False is certain; true is a maybe. The engine limits itself to the block
/// cache internally, so despite the name this does not block — and the column
/// is opened asking for that anyway, in case that changes.
#[implement(super::Map)]
pub(crate) fn maybe_exists<K>(&self, key: &K) -> bool
where
    K: AsRef<[u8]> + ?Sized,
{
    self.db
        .db
        .key_may_exist_cf_opt(&self.cf(), key, &self.cache_read_options)
}
