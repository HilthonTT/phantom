use std::{fmt::Debug, sync::Arc};

use phantom_core::{Result, implement};
use phantom_database::{Deserialized, Engine, Interfix, Map};
use ruma::{OwnedUserId, RoomId, UserId};
use serde::Serialize;

use crate::{
    Dep,
    rooms::{
        self,
        short::{ShortRoomId, ShortStateHash},
    },
    server_state,
};

pub struct Service {
    db: Data,
    services: Services,
}

struct Data {
    engine: Arc<Engine>,
    userroomid_notificationcount: Arc<Map>,
    userroomid_highlightcount: Arc<Map>,
    roomuserid_lastnotificationread: Arc<Map>,
    roomsynctoken_shortstatehash: Arc<Map>,
}

struct Services {
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                engine: args.db.engine.clone(),
                userroomid_notificationcount: args.db["userroomid_notificationcount"].clone(),
                userroomid_highlightcount: args.db["userroomid_highlightcount"].clone(),
                roomuserid_lastnotificationread: args.db["roomuserid_lastnotificationread"].clone(),
                roomsynctoken_shortstatehash: args.db["roomsynctoken_shortstatehash"].clone(),
            },
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) {
    let userroom_id = (user_id, room_id);
    self.db
        .userroomid_highlightcount
        .put(userroom_id, 0_u64)
        .ok();
    self.db
        .userroomid_notificationcount
        .put(userroom_id, 0_u64)
        .ok();

    let roomuser_id = (room_id, user_id);
    let count = self
        .services
        .server_state
        .next_count()
        .expect("the counter is available");
    self.db
        .roomuserid_lastnotificationread
        .put(roomuser_id, count)
        .ok();
}

#[implement(Service)]
pub async fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(&self.db.userroomid_notificationcount, &(user_id, room_id)).await
}

#[implement(Service)]
pub async fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(&self.db.userroomid_highlightcount, &(user_id, room_id)).await
}

#[implement(Service)]
pub async fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(
        &self.db.roomuserid_lastnotificationread,
        &(room_id, user_id),
    )
    .await
}

async fn stored_count<K>(map: &Arc<Map>, key: &K) -> u64
where
    K: Serialize + ?Sized + Debug,
{
    map.qry(key).await.deserialized().unwrap_or(0)
}

#[implement(Service)]
pub async fn associate_token_shortstatehash(
    &self,
    room_id: &RoomId,
    token: u64,
    shortstatehash: ShortStateHash,
) {
    let shortroomid = self
        .services
        .short
        .get_shortroomid(room_id)
        .await
        .expect("room exists");

    let _cork = self.db.engine.cork_guard();
    let key: &[u64] = &[shortroomid, token];
    self.db
        .roomsynctoken_shortstatehash
        .put(key, shortstatehash)
        .ok();
}

#[implement(Service)]
pub async fn get_token_shortstatehash(
    &self,
    room_id: &RoomId,
    token: u64,
) -> Result<ShortStateHash> {
    let shortroomid = self.services.short.get_shortroomid(room_id).await?;

    let key: &[u64] = &[shortroomid, token];
    self.db
        .roomsynctoken_shortstatehash
        .qry(key)
        .await
        .deserialized()
}

#[implement(Service)]
#[tracing::instrument(skip(self, users), level = "debug")]
pub(super) async fn delete_room_notification_state(
    &self,
    room_id: &RoomId,
    shortroomid: ShortRoomId,
    users: &[OwnedUserId],
) {
    let _cork = self.db.engine.cork_guard();

    for user_id in users {
        let userroom_id = (user_id, room_id);

        self.db.userroomid_notificationcount.del(userroom_id).ok();
        self.db.userroomid_highlightcount.del(userroom_id).ok();
    }

    self.db
        .roomuserid_lastnotificationread
        .del_prefix(&(room_id, Interfix))
        .await;

    self.db
        .roomsynctoken_shortstatehash
        .del_prefix(&shortroomid)
        .await;
}
