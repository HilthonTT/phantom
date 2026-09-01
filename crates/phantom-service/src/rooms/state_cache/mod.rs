//! Who is in a room, and which rooms a user is in.
//!
//! Membership is state like any other, and [`state_accessor`] can read it out
//! of a room's state at any version. That is the wrong shape for almost every
//! question actually asked of it: sync wants the rooms one user is joined to,
//! federation wants the servers to send an event to, and both want the answer
//! without loading a room's state at all. So the current membership is kept
//! denormalized here, indexed both ways round — `roomuserid_*` to walk a
//! room's members, `userroomid_*` to walk a user's rooms — and written
//! through [`update_membership`] whenever a membership event lands.
//!
//! Nothing here is authoritative. The state is, and a disagreement means the
//! index is stale rather than that the membership changed.
//!
//! [`state_accessor`]: crate::rooms::state_accessor
//! [`update_membership`]: Service::update_membership

mod members;
mod membership;
mod servers;
mod user_rooms;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use futures::{Stream, StreamExt, future::join5, pin_mut, stream::iter};
use phantom_core::{
    Result, is_not_empty,
    result::LogErr,
    server::Server,
    set,
    stream::{IterStream, ReadyExt, TryIgnore},
};
use phantom_database::{Deserialized, Ignore, Interfix, Json, Map, serialize_key};
use ruma::{
    OwnedRoomId, OwnedServerName, RoomId, ServerName, UserId,
    events::{
        AnyStrippedStateEvent, AnySyncStateEvent, GlobalAccountDataEventType,
        RoomAccountDataEventType, StateEventType,
        direct::DirectEvent,
        room::{
            create::RoomCreateEventContent,
            member::{MembershipState, RoomMemberEventContent},
            power_levels::RoomPowerLevelsEventContent,
        },
    },
    int,
    serde::Raw,
};

use crate::{Dep, account_data, appservice::RegistrationInfo, rooms, server_state, users};

pub struct Service {
    /// Whether an appservice is in a room, by room and then registration id.
    ///
    /// Answering it means scanning a room's members against the appservice's
    /// user namespace, and it is asked once per appservice per event.
    appservice_in_room_cache: AppServiceInRoomCache,
    services: Services,
    db: Data,
}

struct Services {
    server: Arc<Server>,
    account_data: Dep<account_data::Service>,
    metadata: Dep<rooms::metadata::Service>,
    server_state: Dep<server_state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    users: Dep<users::Service>,
}

struct Data {
    roomid_invitedcount: Arc<Map>,
    roomid_inviteviaservers: Arc<Map>,
    roomid_joinedcount: Arc<Map>,
    roomserverids: Arc<Map>,
    roomuserid_invitecount: Arc<Map>,
    roomuserid_joined: Arc<Map>,
    roomuserid_knockedcount: Arc<Map>,
    roomuserid_leftcount: Arc<Map>,
    roomuseroncejoinedids: Arc<Map>,
    serverroomids: Arc<Map>,
    userroomid_invitestate: Arc<Map>,
    userroomid_joined: Arc<Map>,
    userroomid_knockedstate: Arc<Map>,
    userroomid_leftstate: Arc<Map>,
}

type AppServiceInRoomCache = RwLock<HashMap<OwnedRoomId, HashMap<String, bool>>>;
type StrippedStateEventItem = (OwnedRoomId, Vec<Raw<AnyStrippedStateEvent>>);
type SyncStateEventItem = (OwnedRoomId, Vec<Raw<AnySyncStateEvent>>);

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            appservice_in_room_cache: RwLock::new(HashMap::new()),
            services: Services {
                server: args.server.clone(),
                account_data: args.depend::<account_data::Service>("account_data"),
                metadata: args.depend::<rooms::metadata::Service>("rooms::metadata"),
                server_state: args.depend::<server_state::Service>("server_state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                users: args.depend::<users::Service>("users"),
            },
            db: Data {
                roomid_invitedcount: args.db["roomid_invitedcount"].clone(),
                roomid_inviteviaservers: args.db["roomid_inviteviaservers"].clone(),
                roomid_joinedcount: args.db["roomid_joinedcount"].clone(),
                roomserverids: args.db["roomserverids"].clone(),
                roomuserid_invitecount: args.db["roomuserid_invitecount"].clone(),
                roomuserid_joined: args.db["roomuserid_joined"].clone(),
                roomuserid_knockedcount: args.db["roomuserid_knockedcount"].clone(),
                roomuserid_leftcount: args.db["roomuserid_leftcount"].clone(),
                roomuseroncejoinedids: args.db["roomuseroncejoinedids"].clone(),
                serverroomids: args.db["serverroomids"].clone(),
                userroomid_invitestate: args.db["userroomid_invitestate"].clone(),
                userroomid_joined: args.db["userroomid_joined"].clone(),
                userroomid_knockedstate: args.db["userroomid_knockedstate"].clone(),
                userroomid_leftstate: args.db["userroomid_leftstate"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Whether an appservice is party to a room, by its own user being joined
    /// or by any member falling in its user namespace.
    #[tracing::instrument(level = "trace", skip_all)]
    pub async fn appservice_in_room(
        &self,
        room_id: &RoomId,
        appservice: &RegistrationInfo,
    ) -> bool {
        if let Some(cached) = self
            .appservice_in_room_cache
            .read()
            .expect("locked for reading")
            .get(room_id)
            .and_then(|map| map.get(&appservice.registration.id))
            .copied()
        {
            return cached;
        }

        let bridge_user_id = appservice.sender_user(self.services.server_state.server_name());

        let Ok(bridge_user_id) = bridge_user_id.log_err() else {
            return false;
        };

        let in_room = self.is_joined(&bridge_user_id, room_id).await
            || self
                .room_members(room_id)
                .ready_any(|user_id| appservice.users.is_match(user_id.as_str()))
                .await;

        self.appservice_in_room_cache
            .write()
            .expect("locked for writing")
            .entry(room_id.into())
            .or_default()
            .insert(appservice.registration.id.clone(), in_room);

        in_room
    }

    #[must_use]
    pub fn get_appservice_in_room_cache_usage(&self) -> (usize, usize) {
        let cache = self
            .appservice_in_room_cache
            .read()
            .expect("locked for reading");

        (cache.len(), cache.capacity())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    pub fn clear_appservice_in_room_cache(&self) {
        self.appservice_in_room_cache
            .write()
            .expect("locked for writing")
            .clear();
    }

    /// Recounts a room's members and reconciles the server list with them.
    ///
    /// The counts are denormalized, so they are recomputed rather than
    /// adjusted: a missed increment would otherwise be wrong for as long as
    /// the room exists.
    #[tracing::instrument(level = "debug", skip(self))]
    pub async fn update_joined_count(&self, room_id: &RoomId) {
        let mut joinedcount = 0_u64;
        let mut joined_servers = HashSet::new();

        self.room_members(room_id)
            .ready_for_each(|joined| {
                joined_servers.insert(joined.server_name().to_owned());
                joinedcount = joinedcount.saturating_add(1);
            })
            .await;

        let invitedcount: u64 = self
            .room_members_invited(room_id)
            .count()
            .await
            .try_into()
            .unwrap_or(0);

        self.db
            .roomid_joinedcount
            .raw_put(room_id, joinedcount)
            .ok();
        self.db
            .roomid_invitedcount
            .raw_put(room_id, invitedcount)
            .ok();

        self.room_servers(room_id)
            .ready_for_each(|old_joined_server| {
                if joined_servers.remove(old_joined_server) {
                    return;
                }

                self.db.roomserverids.del((room_id, old_joined_server)).ok();
                self.db.serverroomids.del((old_joined_server, room_id)).ok();
            })
            .await;

        for server in &joined_servers {
            self.db.roomserverids.put_raw((room_id, server), []).ok();
            self.db.serverroomids.put_raw((server, room_id), []).ok();
        }

        self.appservice_in_room_cache
            .write()
            .expect("locked for writing")
            .remove(room_id);
    }

    /// The next value of the event counter, which orders a membership change
    /// against everything else that happened.
    fn next_count(&self) -> u64 {
        self.services
            .server_state
            .next_count()
            .expect("the counter is available")
    }
}

/// Which membership a `mark_as_*` write is keeping, for
/// [`Service::clear_other_memberships`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Membership {
    Joined,
    Invited,
    Knocked,
    Left,
}

/// The key one `roomuseroncejoinedids` entry is written under.
///
/// Spelled once rather than at each of the three uses, so that the writer,
/// the point read and the prefix scan cannot drift apart. Building it the
/// other way round is not an error anywhere: the scan in
/// [`Service::room_useroncejoined`] would simply return nothing forever,
/// since a user id can never fall under a room id's prefix.
fn once_joined_key<'a>(room_id: &'a RoomId, user_id: &'a UserId) -> (&'a RoomId, &'a UserId) {
    (room_id, user_id)
}

/// The prefix every [`once_joined_key`] for one room falls under.
fn once_joined_prefix(room_id: &RoomId) -> (&RoomId, Interfix) {
    (room_id, Interfix)
}

/// The rooms in a column of stripped state keyed by (user, room), with that
/// state deserialized.
fn stripped_rooms<'a>(
    map: &'a Arc<Map>,
    user_id: &'a UserId,
) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
    type KeyVal<'a> = ((Ignore, &'a str), Raw<Vec<Raw<AnyStrippedStateEvent>>>);

    let prefix = (user_id, Interfix);
    map.stream_prefix(&prefix)
        .ignore_err()
        .map(|((_, room_id), state): KeyVal<'_>| {
            let room_id = <&RoomId>::try_from(room_id).expect("valid room id in db");
            Ok((room_id.to_owned(), state.deserialize()?))
        })
        .ignore_err()
}

#[cfg(test)]
mod tests {
    use phantom_database::serialize_to_vec;
    use ruma::{RoomId, UserId};

    use super::{once_joined_key, once_joined_prefix};

    /// `roomuseroncejoinedids` is written a key at a time by
    /// [`Service::mark_as_once_joined`] and read a room at a time by
    /// [`Service::room_useroncejoined`], so the key one builds has to fall
    /// inside the prefix the other scans. Swapping the halves is not an error
    /// anywhere — the scan just comes back empty forever.
    ///
    /// [`Service::mark_as_once_joined`]: super::Service::mark_as_once_joined
    /// [`Service::room_useroncejoined`]: super::Service::room_useroncejoined
    #[test]
    fn once_joined_keys_fall_under_the_room_prefix() {
        let user_id = UserId::parse("@alice:phantom.test").expect("valid user id");
        let room_id = RoomId::parse("!room:phantom.test").expect("valid room id");

        let key = serialize_to_vec(once_joined_key(&room_id, &user_id)).expect("serialized");
        let prefix = serialize_to_vec(once_joined_prefix(&room_id)).expect("serialized");

        assert!(
            key.starts_with(&prefix),
            "a once-joined key must be reachable from the room prefix",
        );
    }
}
