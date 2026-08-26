//! Emptying a column.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement, utils::stream::TryReadyExt};

use crate::keyval::Key;

/// Deletes every entry in the column, and reports what it deleted.
///
/// Deletion runs against the entries the iteration finds, which is a view of
/// the column as it stood when this was called: anything written after that
/// may or may not be seen. Nothing stops a concurrent writer, so a column
/// still being written to will not be empty when this finishes.
///
/// A failed delete ends the stream — the failures are of a kind that will
/// recur — but the deletes already made stand.
#[implement(super::Map)]
#[tracing::instrument(level = "trace", fields(%self))]
pub fn for_clear(self: &Arc<Self>) -> impl Stream<Item = Result<Key<'_>>> + Send + use<'_> {
    self.raw_keys()
        .ready_and_then(|key| self.remove(key).map(|()| key))
}

/// [`Self::for_clear`], discarding what it deleted and whether it worked.
///
/// Reach for `for_clear` where either matters.
#[implement(super::Map)]
#[inline]
pub async fn clear(self: &Arc<Self>) {
    self.for_clear().count().await;
}
