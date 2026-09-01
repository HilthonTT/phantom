//! What the server knows about a room as a whole.
//!
//! Whether it exists at all — which is a question about its timeline, not
//! about a record here — and the two administrative flags an operator sets on
//! it: banned, which refuses the room outright, and disabled, which stops
//! federating it without forgetting it.

use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{Result, implement, stream::TryIgnore};
use phantom_database::{Map, serialize_to_vec};
use ruma::RoomId;

use crate::{Dep, rooms};

pub struct Service {
    db: Data,
    services: Services,
}

struct Data {
    disabledroomids: Arc<Map>,
    bannedroomids: Arc<Map>,
    roomid_shortroomid: Arc<Map>,
    pduid_pdu: Arc<Map>,
}

struct Services {
    short: Dep<rooms::short::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                disabledroomids: args.db["disabledroomids"].clone(),
                bannedroomids: args.db["bannedroomids"].clone(),
                roomid_shortroomid: args.db["roomid_shortroomid"].clone(),
                pduid_pdu: args.db["pduid_pdu"].clone(),
            },
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn exists(&self, room_id: &RoomId) -> bool {
    let Ok(shortroomid) = self.services.short.get_shortroomid(room_id).await else {
        return false;
    };

    let prefix = serialize_to_vec(shortroomid).expect("failed to serialize prefix");

    self.db
        .pduid_pdu
        .raw_keys_prefix(&prefix)
        .ignore_err()
        .next()
        .await
        .is_some()
}

/// Every room the server has a record of.
#[implement(Service)]
pub fn iter_ids(&self) -> impl Stream<Item = &RoomId> + Send + '_ {
    room_ids(&self.db.roomid_shortroomid)
}

/// Bans the room, or lifts the ban.
#[implement(Service)]
#[inline]
pub fn ban_room(&self, room_id: &RoomId, banned: bool) {
    if banned {
        self.db.bannedroomids.insert(room_id, []).ok();
    } else {
        self.db.bannedroomids.remove(room_id).ok();
    }
}

/// Every room an operator has banned.
#[implement(Service)]
pub fn list_banned_rooms(&self) -> impl Stream<Item = &RoomId> + Send + '_ {
    room_ids(&self.db.bannedroomids)
}

/// The keys of a column keyed by room id, as room ids.
fn room_ids(map: &Arc<Map>) -> impl Stream<Item = &RoomId> + Send + '_ {
    map.keys::<&str>()
        .ignore_err()
        .map(|room_id| <&RoomId>::try_from(room_id).expect("valid room id in db"))
}

#[implement(Service)]
#[inline]
pub async fn is_disabled(&self, room_id: &RoomId) -> bool {
    self.db.disabledroomids.get(room_id).await.is_ok()
}

#[implement(Service)]
#[inline]
pub async fn is_banned(&self, room_id: &RoomId) -> bool {
    self.db.bannedroomids.get(room_id).await.is_ok()
}
