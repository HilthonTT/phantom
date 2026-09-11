use std::{fmt::Debug, sync::Arc};

use futures::{Stream, StreamExt, TryStreamExt};
use phantom_core::{
    Result, implement,
    stream::{IterStream, WidebandExt, automatic_amplification, automatic_width},
};
use serde::Serialize;

use crate::{Handle, codec::serialize::serialize_to, keyval::KeyBuf, pool};

pub trait Qry<'a, K, S>
where
    S: Stream<Item = K> + Send + 'a,
    K: Serialize + Debug,
{
    fn qry(self, map: &'a Arc<super::Map>) -> impl Stream<Item = Result<Handle<'a>>> + Send + 'a;
}

impl<'a, K, S> Qry<'a, K, S> for S
where
    Self: 'a,
    S: Stream<Item = K> + Send + 'a,
    K: Serialize + Debug + 'a,
{
    #[inline]
    fn qry(self, map: &'a Arc<super::Map>) -> impl Stream<Item = Result<Handle<'a>>> + Send + 'a {
        map.qry_batch(self)
    }
}

#[implement(super::Map)]
#[tracing::instrument(skip(self, keys), level = "trace")]
pub fn qry_batch<'a, S, K>(
    self: &'a Arc<Self>,
    keys: S,
) -> impl Stream<Item = Result<Handle<'a>>> + Send + 'a
where
    S: Stream<Item = K> + Send + 'a,
    K: Serialize + Debug + 'a,
{
    keys.ready_chunks(automatic_amplification())
        .widen_then(automatic_width(), |chunk| {
            let keys = chunk
                .iter()
                .map(serialize_to::<KeyBuf, _>)
                .map(|result| result.expect("failed to serialize query key"))
                .collect();

            self.db.pool.execute_get(pool::Get {
                map: self.clone(),
                key: keys,
                res: None,
            })
        })
        .map_ok(|results| results.into_iter().stream())
        .try_flatten()
}
