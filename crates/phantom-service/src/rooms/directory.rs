//! Which rooms this server publishes in its public room directory.
//!
//! The column is the whole of it: a room id is present when the room is
//! published and absent when it is not, so there is no value to store beside
//! the key.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{Result, err, implement, stream::TryIgnore};
use phantom_database::Map;
use ruma::{RoomId, api::client::room::Visibility};

pub struct Service {
    db: Data,
}

struct Data {
    publicroomids: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                publicroomids: args.db["publicroomids"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Publishes `room_id` in the directory.
///
/// The write is fallible here rather than queued, so the caller is the one
/// that decides what a failed publish means for the request that asked for
/// it.
#[implement(Service)]
pub fn set_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.insert(room_id, [])
}

/// Removes `room_id` from the directory. Removing one that was never
/// published is not an error.
#[implement(Service)]
pub fn set_not_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.remove(room_id)
}

/// Every published room, in key order.
///
/// The column stores the id as its own bytes, so the room ids borrow from the
/// iterator rather than being allocated one by one; validating them back into
/// `&RoomId` is what the parse below is for.
#[implement(Service)]
pub fn public_rooms(&self) -> impl Stream<Item = &RoomId> + Send {
    self.db
        .publicroomids
        .keys::<&str>()
        .map(|room_id| {
            room_id.and_then(|room_id| {
                <&RoomId>::try_from(room_id)
                    .map_err(|e| err!(Database("Invalid room id in publicroomids: {e}")))
            })
        })
        .ignore_err()
}

/// Whether `room_id` is published.
#[implement(Service)]
pub async fn is_public_room(&self, room_id: &RoomId) -> bool {
    self.visibility(room_id).await == Visibility::Public
}

/// The directory visibility of `room_id`, which is `Private` for a room that
/// was never published as much as for one that was unpublished.
#[implement(Service)]
pub async fn visibility(&self, room_id: &RoomId) -> Visibility {
    if self.db.publicroomids.get(room_id).await.is_ok() {
        Visibility::Public
    } else {
        Visibility::Private
    }
}
