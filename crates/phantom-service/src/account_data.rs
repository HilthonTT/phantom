//! The account data a user has set, global or per-room.
//!
//! One event per (user, room, type), where the global scope is the room-less
//! one. Each write is stamped with a number from the server counter and
//! indexed by it, so that a sync can ask for everything a user changed since
//! the token it last saw rather than re-reading the whole account.

use std::sync::Arc;

use futures::{Stream, StreamExt, TryFutureExt};
use phantom_core::{
    Err, Result, err, implement,
    result::LogErr,
    stream::{ReadyExt, TryIgnore},
};
use phantom_database::{Deserialized, Handle, Ignore, Json, Map};
use ruma::{
    RoomId, UserId,
    events::{
        AnyGlobalAccountDataEvent, AnyRoomAccountDataEvent, GlobalAccountDataEventType,
        RoomAccountDataEventType,
    },
    serde::Raw,
};
use serde::{Deserialize, Serialize};

use crate::{Dep, server_state};

pub struct Service {
    services: Services,
    db: Data,
}

struct Data {
    roomuserdataid_accountdata: Arc<Map>,
    roomusertype_roomuserdataid: Arc<Map>,
}

struct Services {
    server_state: Dep<server_state::Service>,
}

/// One account data event still in the form it was stored in.
///
/// The two halves of account data are distinct event enums, but a sync
/// response carries them in one stream and passes them through without
/// looking inside, so they are kept raw and only tagged with which side of
/// the split they came from.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum AnyRawAccountDataEvent {
    Global(Raw<AnyGlobalAccountDataEvent>),
    Room(Raw<AnyRoomAccountDataEvent>),
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
            },
            db: Data {
                roomuserdataid_accountdata: args.db["roomuserdataid_accountdata"].clone(),
                roomusertype_roomuserdataid: args.db["roomusertype_roomuserdataid"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Places one event in the account data of the user and removes the
/// previous entry.
#[allow(clippy::needless_pass_by_value)]
#[implement(Service)]
pub async fn update(
    &self,
    room_id: Option<&RoomId>,
    user_id: &UserId,
    event_type: RoomAccountDataEventType,
    data: &serde_json::Value,
) -> Result<()> {
    if data.get("type").is_none() || data.get("content").is_none() {
        return Err!(Request(InvalidParam(
            "Account data doesn't have all required fields."
        )));
    }

    let count = self
        .services
        .server_state
        .next_count()
        .expect("the counter is available");
    let roomuserdataid = (room_id, user_id, count, &event_type);
    let _ = self
        .db
        .roomuserdataid_accountdata
        .put(roomuserdataid, Json(data));

    let key = (room_id, user_id, &event_type);
    let prev = self.db.roomusertype_roomuserdataid.qry(&key).await;
    let _ = self.db.roomusertype_roomuserdataid.put(key, roomuserdataid);

    // Remove old entry
    if let Ok(prev) = prev {
        let _ = self.db.roomuserdataid_accountdata.remove(&prev);
    }

    Ok(())
}

/// Searches the room account data for a specific kind.
#[implement(Service)]
pub async fn get_global<T>(&self, user_id: &UserId, kind: GlobalAccountDataEventType) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    self.get_raw(None, user_id, &kind.to_string())
        .await
        .deserialized()
}

/// Searches the global account data for a specific kind.
#[implement(Service)]
pub async fn get_room<T>(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
    kind: RoomAccountDataEventType,
) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    self.get_raw(Some(room_id), user_id, &kind.to_string())
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn get_raw(
    &self,
    room_id: Option<&RoomId>,
    user_id: &UserId,
    kind: &str,
) -> Result<Handle<'_>> {
    let key = (room_id, user_id, kind.to_owned());
    self.db
        .roomusertype_roomuserdataid
        .qry(&key)
        .and_then(|roomuserdataid| self.db.roomuserdataid_accountdata.get(&roomuserdataid))
        .await
}

/// Returns all changes to the account data that happened after `since`.
#[implement(Service)]
pub fn changes_since<'a>(
    &'a self,
    room_id: Option<&'a RoomId>,
    user_id: &'a UserId,
    since: u64,
    to: Option<u64>,
) -> impl Stream<Item = AnyRawAccountDataEvent> + Send + 'a {
    // The identifiers come back out of the key as the strings they were
    // written as: the database layer knows nothing of ruma's types, and
    // re-parsing them only to compare them would cost more than comparing the
    // bytes.
    type Key<'a> = (Option<&'a str>, &'a str, u64, Ignore);

    let room_id_str = room_id.map(RoomId::as_str);
    let user_id_str = user_id.as_str();

    // Skip the data that's exactly at since, because we sent that last time
    let first_possible = (room_id, user_id, since.saturating_add(1));

    self.db
        .roomuserdataid_accountdata
        .stream_from(&first_possible)
        .ignore_err()
        .ready_take_while(move |((room_id_, user_id_, count, _), _): &(Key<'_>, _)| {
            room_id_str == *room_id_ && user_id_str == *user_id_ && to.is_none_or(|to| *count <= to)
        })
        .map(move |(_, v)| {
            match room_id {
                Some(_) => serde_json::from_slice::<Raw<AnyRoomAccountDataEvent>>(v)
                    .map(AnyRawAccountDataEvent::Room),
                None => serde_json::from_slice::<Raw<AnyGlobalAccountDataEvent>>(v)
                    .map(AnyRawAccountDataEvent::Global),
            }
            .map_err(|e| err!(Database("Database contains invalid account data: {e}")))
            .log_err()
        })
        .ignore_err()
}
