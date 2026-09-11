use std::{convert::AsRef, fmt::Debug, io::Write, sync::Arc};

use arrayvec::ArrayVec;
use phantom_core::{Result, implement};
use serde::Serialize;

use crate::{Handle, codec::serialize::serialize, keyval::KeyBuf};

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
