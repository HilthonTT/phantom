//! Deleting every entry under a prefix.
//!
//! The pattern this replaces — scan the keys, remove each one — was written out
//! by hand at every call site that needed it, which meant every call site also
//! had to remember to ignore the scan's errors and to hold a cork around the
//! run. Both are here instead.

use std::{convert::AsRef, fmt::Debug, sync::Arc};

use phantom_core::{
    implement,
    stream::{ReadyExt, TryIgnore},
};
use serde::Serialize;

use crate::keyval::serialize_key;

/// Deletes every entry whose key begins with `prefix`.
///
/// The scan and the deletions are interleaved over one iterator, so a write
/// that lands after the cursor has passed its key survives — this clears what
/// was there when it started, not whatever is there when it ends.
///
/// The deletions go out under one cork, so the run costs one flush rather than
/// one per key. They are not atomic: a failure part-way leaves the keys already
/// removed removed. Use [`Txn`](crate::Txn) where all-or-nothing matters.
#[implement(super::Map)]
#[tracing::instrument(level = "trace", skip(self))]
pub async fn del_prefix<P>(self: &Arc<Self>, prefix: &P)
where
    P: Serialize + ?Sized + Debug,
{
    let prefix = serialize_key(prefix).expect("failed to serialize prefix");

    self.raw_del_prefix(&prefix).await;
}

/// [`Self::del_prefix`] with the prefix already serialized.
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
