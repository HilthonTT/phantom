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

#[implement(Service)]
pub fn set_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.insert(room_id, [])
}

#[implement(Service)]
pub fn set_not_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.remove(room_id)
}

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

#[implement(Service)]
pub async fn is_public_room(&self, room_id: &RoomId) -> bool {
    self.visibility(room_id).await == Visibility::Public
}

#[implement(Service)]
pub async fn visibility(&self, room_id: &RoomId) -> Visibility {
    if self.db.publicroomids.get(room_id).await.is_ok() {
        Visibility::Public
    } else {
        Visibility::Private
    }
}
