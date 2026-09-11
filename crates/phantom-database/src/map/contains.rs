use std::{convert::AsRef, fmt::Debug, future::Future, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use futures::FutureExt;
use phantom_core::{Result, err, future::TryExt, implement, result::FlatOk};
use serde::Serialize;

use crate::{codec::serialize::serialize, keyval::KeyBuf};

#[implement(super::Map)]
#[inline]
pub fn contains<K>(self: &Arc<Self>, key: &K) -> impl Future<Output = bool> + Send + use<'_, K>
where
    K: Serialize + ?Sized + Debug,
{
    let mut buf = KeyBuf::new();

    self.bcontains(key, &mut buf)
}

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
    let key = serialize(buf, key).expect("failed to serialize query key");

    self.exists(key).is_ok()
}

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

#[implement(super::Map)]
pub(crate) fn maybe_exists<K>(&self, key: &K) -> bool
where
    K: AsRef<[u8]> + ?Sized,
{
    self.db
        .db
        .key_may_exist_cf_opt(&self.cf(), key, &self.cache_read_options)
}
