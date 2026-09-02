//! What one user has going on in one room.
//!
//! The unread counters a client renders as a badge — notifications and, of
//! them, the ones that mention the user — plus the read marker they are
//! counted from. Also the sync token a client last held for the room, mapped
//! to the room's state at that moment, which is what lets the next sync send a
//! delta rather than the whole state.

use std::{fmt::Debug, sync::Arc};

use phantom_core::{Result, implement};
use phantom_database::{Deserialized, Engine, Map};
use ruma::{RoomId, UserId};
use serde::Serialize;

use crate::{
    Dep,
    rooms::{self, short::ShortStateHash},
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

/// How many notifications the user has waiting in the room.
#[implement(Service)]
pub async fn notification_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(&self.db.userroomid_notificationcount, &(user_id, room_id)).await
}

/// How many of those notifications mention the user.
#[implement(Service)]
pub async fn highlight_count(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(&self.db.userroomid_highlightcount, &(user_id, room_id)).await
}

/// The counter value the user's read marker was last moved to.
#[implement(Service)]
pub async fn last_notification_read(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    stored_count(
        &self.db.roomuserid_lastnotificationread,
        &(room_id, user_id),
    )
    .await
}

/// A counter column's value, where never having been written counts as zero.
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
