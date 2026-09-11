use std::{convert::AsRef, fmt::Debug, sync::Arc};

use phantom_core::{
    implement,
    stream::{ReadyExt, TryIgnore},
};
use serde::Serialize;

use crate::keyval::serialize_key;

#[implement(super::Map)]
#[tracing::instrument(level = "trace", skip(self))]
pub async fn del_prefix<P>(self: &Arc<Self>, prefix: &P)
where
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize prefix");

    self.raw_del_prefix(&prefix).await;
}

#[implement(super::Map)]
#[tracing::instrument(level = "trace", skip(self, prefix))]
pub async fn raw_del_prefix<P>(self: &Arc<Self>, prefix: &P)
where
    P: AsRef<[u8]> + ?Sized + Debug + Sync,
{
    let _cork = self.db().cork_guard();

    self.raw_keys_prefix(prefix)
        .ignore_err()
        .ready_for_each(|key| {
            self.remove(key).ok();
        })
        .await;
}
