use std::{convert::AsRef, fmt::Debug, future::Future, sync::Arc};

use futures::StreamExt;
use phantom_core::implement;
use serde::Serialize;

use crate::{
    cursor::FORWARD,
    keyval::{Key, serialize_key},
};

#[implement(super::Map)]
#[inline]
pub fn count(self: &Arc<Self>) -> impl Future<Output = usize> + Send + use<'_> {
    self.raw_keys().count()
}

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
