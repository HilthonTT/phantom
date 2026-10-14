use std::{fmt::Debug, sync::Arc};

use futures::StreamExt;

use phantom_core::{
    Result, implement,
    stream::{ReadyExt, TryIgnore},
    sync::MutexMap,
};
use phantom_database::{Deserialized, Engine, Ignore, Interfix, Map, table};
use ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId};
use serde::Serialize;

use crate::{
    Dep,
    ops::server_state,
    rooms::{
        self,
        short::{ShortRoomId, ShortStateHash},
    },
};

pub struct Service {
    db: Data,
    services: Services,

    /// Serializes a user's count writes. Raising a count is a read followed
    /// by a write, and a reset landing between the two would be overwritten
    /// by the stale sum, leaving a room unread that the user just read.
    count_mutex: MutexMap<OwnedUserId, ()>,
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
    state_cache: Dep<rooms::state_cache::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                engine: args.db.engine.clone(),
                userroomid_notificationcount: args.db[table::USERROOMID_NOTIFICATIONCOUNT].clone(),
                userroomid_highlightcount: args.db[table::USERROOMID_HIGHLIGHTCOUNT].clone(),
                roomuserid_lastnotificationread: args.db[table::ROOMUSERID_LASTNOTIFICATIONREAD]
                    .clone(),
                roomsynctoken_shortstatehash: args.db[table::ROOMSYNCTOKEN_SHORTSTATEHASH].clone(),
            },
            services: Services {
                server_state: args.depend::<server_state::Service>(),
                short: args.depend::<rooms::short::Service>(),
                state_cache: args.depend::<rooms::state_cache::Service>(),
            },
            count_mutex: MutexMap::new(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Zeroes the user's counts in the room and stamps when it happened.
///
/// The stamp is a fresh count, so it is above every event already in the
/// room: `GET /notifications` reports an event as read when it is below the
/// stamp, which keeps that flag in step with the counts sync serves.
#[implement(Service)]
pub async fn reset_notification_counts(&self, user_id: &UserId, room_id: &RoomId) {
    let _lock = self.count_mutex.lock(user_id).await;

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

/// Counts one more notifying event for the user in the room, and one more
/// highlight when `highlight` is set. `event_count` is the event's position
/// in the timeline: an event the user has already read past, by a receipt
/// that arrived between the event becoming visible and this count, is not
/// counted, or the room would stay unread after it was read.
#[implement(Service)]
pub async fn increment_notification_counts(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    event_count: u64,
    highlight: bool,
) {
    let _lock = self.count_mutex.lock(user_id).await;

    if self.last_notification_read(user_id, room_id).await > event_count {
        return;
    }

    let userroom_id = (user_id, room_id);
    increment(&self.db.userroomid_notificationcount, userroom_id).await;

    if highlight {
        increment(&self.db.userroomid_highlightcount, userroom_id).await;
    }
}

async fn increment(map: &Arc<Map>, key: (&UserId, &RoomId)) {
    let old = stored_count(map, &key).await;
    map.put(key, old.saturating_add(1)).ok();
}

/// The user's unread notifications across every room they are joined to:
/// the badge a push gateway shows.
///
/// Rooms the user has left keep their last count until the room is deleted,
/// so only joined rooms are summed; otherwise a room left unread would hold
/// the badge up for good.
#[implement(Service)]
pub async fn global_notification_count(&self, user_id: &UserId) -> u64 {
    let counts: Vec<_> = self
        .db
        .userroomid_notificationcount
        .stream_prefix(&(user_id, Interfix))
        .ignore_err()
        .ready_filter_map(|((_, room_id), count): ((Ignore, &str), u64)| {
            let room_id = OwnedRoomId::try_from(room_id).ok()?;

            (count > 0).then_some((room_id, count))
        })
        .boxed()
        .collect()
        .await;

    let mut total = 0_u64;
    for (room_id, count) in counts {
        if self.services.state_cache.is_joined(user_id, &room_id).await {
            total = total.saturating_add(count);
        }
    }

    total
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
