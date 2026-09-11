use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement, stream::TryReadyExt};

use crate::keyval::Key;

#[implement(super::Map)]
#[tracing::instrument(level = "trace", fields(%self))]
pub fn for_clear(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_> {
    self.raw_keys()
        .ready_and_then(|key| self.remove(key).map(|()| key))
}

#[implement(super::Map)]
#[inline]
pub async fn clear(self: &Arc<Self>) {
    self.for_clear().count().await;
}
