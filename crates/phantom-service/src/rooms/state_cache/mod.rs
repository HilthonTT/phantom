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
    /// Brings the membership indexes in line with a membership event.
    ///
    /// This is the way in: the `mark_as_*` functions below are the individual
    /// writes it is made of, and skip everything this does around them.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            %room_id,
            %user_id,
            %sender,
            ?membership_event,
        ),
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn update_membership(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        membership_event: RoomMemberEventContent,
        sender: &UserId,
        last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
        invite_via: Option<Vec<OwnedServerName>>,
        update_joined_count: bool,
    ) -> Result<()> {
        let membership = membership_event.membership;

        // A remote user is recorded locally as a deactivated account, which is
        // what gives the rest of the server somewhere to hang their profile
        // and their device list off.
        if !self.services.server_state.user_is_local(user_id)
            && !self.services.users.exists(user_id).await
        {
            self.services.users.create(user_id, None)?;
        }

        match &membership {
            MembershipState::Join => {
                // Only on the first join: an upgraded room inherits the
                // settings the user had in the room it replaced.
                if !self.once_joined(user_id, room_id).await {
                    self.mark_as_once_joined(user_id, room_id);

                    if let Ok(Some(predecessor)) = self
                        .services
                        .state_accessor
                        .room_state_get_content(room_id, &StateEventType::RoomCreate, "")
                        .await
                        .map(|content: RoomCreateEventContent| content.predecessor)
                    {
                        self.copy_room_settings(&predecessor.room_id, room_id, user_id)
                            .await?;
                    }
                }

                self.mark_as_joined(user_id, room_id);
            }
            MembershipState::Invite => {
                // An invite from someone the invitee ignores is dropped
                // rather than recorded, so it never reaches their sync.
                if self.services.users.user_is_ignored(sender, user_id).await {
                    return Ok(());
                }

                self.mark_as_invited(user_id, room_id, last_state, invite_via)
                    .await;
            }
            MembershipState::Leave | MembershipState::Ban => {
                self.mark_as_left(user_id, room_id);

                if self.services.server_state.user_is_local(user_id)
                    && (self.services.server.config.forget_forced_upon_leave
                        || self.services.metadata.is_banned(room_id).await
                        || self.services.metadata.is_disabled(room_id).await)
                {
                    self.forget(room_id, user_id);
                }
            }
            _ => {}
        }

        if update_joined_count {
            self.update_joined_count(room_id).await;
        }

        Ok(())
    }

    /// Carries a user's per-room settings across a room upgrade.
    ///
    /// Tags and the direct-chat flag only. Push rules are the other thing the
    /// spec asks to be carried over, and are not here because phantom has no
    /// pusher service yet to own them.
    async fn copy_room_settings(
        &self,
        predecessor: &RoomId,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<()> {
        if let Ok(tag_event) = self
            .services
            .account_data
            .get_room::<serde_json::Value>(predecessor, user_id, RoomAccountDataEventType::Tag)
            .await
        {
            self.services
                .account_data
                .update(
                    Some(room_id),
                    user_id,
                    RoomAccountDataEventType::Tag,
                    &tag_event,
                )
                .await
                .ok();
        }

        if let Ok(mut direct_event) = self
            .services
            .account_data
            .get_global::<DirectEvent>(user_id, GlobalAccountDataEventType::Direct)
            .await
        {
            let mut room_ids_updated = false;
            for room_ids in direct_event.content.0.values_mut() {
                if room_ids.iter().any(|r| r == predecessor) {
                    room_ids.push(room_id.to_owned());
                    room_ids_updated = true;
                }
            }

            if room_ids_updated {
                self.services
                    .account_data
                    .update(
                        None,
                        user_id,
                        GlobalAccountDataEventType::Direct.to_string().into(),
                        &serde_json::to_value(&direct_event).expect("to json always works"),
                    )
                    .await?;
            }
        }

        Ok(())
    }

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

    /// Records a user as joined, clearing every other membership.
    ///
    /// One of the raw writes behind [`update_membership`], which is what a
    /// caller should reach for instead: this skips the ignore check, the
    /// upgrade carry-over and the count refresh.
    ///
    /// [`update_membership`]: Self::update_membership
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_joined(&self, user_id: &UserId, room_id: &RoomId) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db.userroomid_joined.insert(&userroom_id, []).ok();
        self.db.roomuserid_joined.insert(&roomuser_id, []).ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Joined);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    /// Records a user as having left, clearing every other membership.
    ///
    /// See [`mark_as_joined`](Self::mark_as_joined) on using this directly.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_left(&self, user_id: &UserId, room_id: &RoomId) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        // The stripped state a client is shown for a room it has left is not
        // recorded yet, so the entry exists only to mark the membership.
        let leftstate = Vec::<Raw<AnySyncStateEvent>>::new();

        self.db
            .userroomid_leftstate
            .raw_put(&userroom_id, Json(leftstate))
            .ok();
        self.db
            .roomuserid_leftcount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Left);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    /// Records a user as knocking, clearing every other membership.
    ///
    /// See [`mark_as_joined`](Self::mark_as_joined) on using this directly.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_knocked(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        knocked_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
    ) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db
            .userroomid_knockedstate
            .raw_put(&userroom_id, Json(knocked_state.unwrap_or_default()))
            .ok();
        self.db
            .roomuserid_knockedcount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Knocked);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    /// Records a user as invited, clearing every other membership.
    ///
    /// See [`mark_as_joined`](Self::mark_as_joined) on using this directly.
    #[tracing::instrument(level = "debug", skip(self, last_state, invite_via))]
    pub async fn mark_as_invited(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
        invite_via: Option<Vec<OwnedServerName>>,
    ) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db
            .userroomid_invitestate
            .raw_put(&userroom_id, Json(last_state.unwrap_or_default()))
            .ok();
        self.db
            .roomuserid_invitecount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Invited);

        if let Some(servers) = invite_via.filter(is_not_empty!()) {
            self.add_servers_invite_via(room_id, servers).await;
        }
    }

    /// Marks a user as having joined at some point, which is what tells a
    /// later join from a first one.
    #[tracing::instrument(level = "debug", skip(self))]
    fn mark_as_once_joined(&self, user_id: &UserId, room_id: &RoomId) {
        let key = once_joined_key(room_id, user_id);
        self.db.roomuseroncejoinedids.put_raw(key, []).ok();
    }

    /// Drops every membership record but the one just written.
    ///
    /// A user has one membership in a room, so setting one is also clearing
    /// the rest; doing that in one place is what keeps the four `mark_as_*`
    /// functions from drifting as a fifth membership is added.
    fn clear_other_memberships(&self, userroom_id: &[u8], roomuser_id: &[u8], keep: Membership) {
        if keep != Membership::Joined {
            self.db.userroomid_joined.remove(userroom_id).ok();
            self.db.roomuserid_joined.remove(roomuser_id).ok();
        }

        if keep != Membership::Invited {
            self.db.userroomid_invitestate.remove(userroom_id).ok();
            self.db.roomuserid_invitecount.remove(roomuser_id).ok();
        }

        if keep != Membership::Left {
            self.db.userroomid_leftstate.remove(userroom_id).ok();
            self.db.roomuserid_leftcount.remove(roomuser_id).ok();
        }

        if keep != Membership::Knocked {
            self.db.userroomid_knockedstate.remove(userroom_id).ok();
            self.db.roomuserid_knockedcount.remove(roomuser_id).ok();
        }
    }

    /// Makes a user forget a room, which drops it from their sync entirely.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn forget(&self, room_id: &RoomId, user_id: &UserId) {
        self.db.userroomid_leftstate.del((user_id, room_id)).ok();
        self.db.roomuserid_leftcount.del((room_id, user_id)).ok();
    }

    /// Every server with a member in this room.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_servers<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a ServerName> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db.roomserverids.keys_prefix(&prefix).ignore_err().map(
            |(_, server): (Ignore, &str)| {
                <&ServerName>::try_from(server).expect("valid server name in db")
            },
        )
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn server_in_room<'a>(&'a self, server: &'a ServerName, room_id: &'a RoomId) -> bool {
        let key = (server, room_id);
        self.db.serverroomids.qry(&key).await.is_ok()
    }

    /// Every room this server knows a given server to be in.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn server_rooms<'a>(
        &'a self,
        server: &'a ServerName,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let prefix = (server, Interfix);
        self.db.serverroomids.keys_prefix(&prefix).ignore_err().map(
            |(_, room_id): (Ignore, &str)| {
                <&RoomId>::try_from(room_id).expect("valid room id in db")
            },
        )
    }

    /// Whether a server shares any room with a user, which is what entitles it
    /// to see that user's profile and device list.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn server_sees_user(&self, server: &ServerName, user_id: &UserId) -> bool {
        self.server_rooms(server)
            .any(|room_id| self.is_joined(user_id, room_id))
            .await
    }

    /// Whether two users share any room.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn user_sees_user(&self, user_a: &UserId, user_b: &UserId) -> bool {
        let get_shared_rooms = self.get_shared_rooms(user_a, user_b);

        pin_mut!(get_shared_rooms);
        get_shared_rooms.next().await.is_some()
    }

    /// The rooms two users are both joined to.
    ///
    /// Both sides come out of the same column in the same order, so this is a
    /// merge of two sorted streams rather than either one collected.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn get_shared_rooms<'a>(
        &'a self,
        user_a: &'a UserId,
        user_b: &'a UserId,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let a = self.rooms_joined(user_a);
        let b = self.rooms_joined(user_b);

        set::intersection_sorted_stream2(a, b)
    }

    /// Every joined member of a room.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_joined
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    /// The joined member count, as maintained by [`update_joined_count`].
    ///
    /// [`update_joined_count`]: Self::update_joined_count
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn room_joined_count(&self, room_id: &RoomId) -> Result<u64> {
        self.db.roomid_joinedcount.get(room_id).await.deserialized()
    }

    /// The invited member count, as maintained by [`update_joined_count`].
    ///
    /// [`update_joined_count`]: Self::update_joined_count
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn room_invited_count(&self, room_id: &RoomId) -> Result<u64> {
        self.db
            .roomid_invitedcount
            .get(room_id)
            .await
            .deserialized()
    }

    /// Every member of a room who is one of ours, deactivated and guests
    /// included.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn local_users_in_room<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        self.room_members(room_id)
            .ready_filter(|user| self.services.server_state.user_is_local(user))
    }

    /// [`local_users_in_room`] narrowed to accounts that can still act.
    ///
    /// [`local_users_in_room`]: Self::local_users_in_room
    #[tracing::instrument(skip(self), level = "trace")]
    pub fn active_local_users_in_room<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        self.local_users_in_room(room_id)
            .filter(|user| self.services.users.is_active(user))
    }

    /// Every user who has ever been joined to a room.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_useroncejoined<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = once_joined_prefix(room_id);
        self.db
            .roomuseroncejoinedids
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    /// Every user invited to a room and not yet joined.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members_invited<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_invitecount
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    /// Every user knocking at a room.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members_knocked<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_knockedcount
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    /// The counter value at which a user was invited, which is what orders the
    /// invite in sync.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_invite_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db
            .roomuserid_invitecount
            .qry(&key)
            .await
            .deserialized()
    }

    /// The counter value at which a user knocked.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_knock_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db
            .roomuserid_knockedcount
            .qry(&key)
            .await
            .deserialized()
    }

    /// The counter value at which a user left.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_left_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db.roomuserid_leftcount.qry(&key).await.deserialized()
    }

    /// Every room a user is joined to.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_joined<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let prefix = (user_id, Interfix);
        self.db
            .userroomid_joined
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, room_id): (Ignore, &str)| {
                <&RoomId>::try_from(room_id).expect("valid room id in db")
            })
    }

    /// Every room a user has been invited to, with the stripped state the
    /// client is shown for it.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_invited<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_invitestate, user_id)
    }

    /// Every room a user is knocking at, with its stripped state.
    #[tracing::instrument(skip(self), level = "trace")]
    pub fn rooms_knocked<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_knockedstate, user_id)
    }

    /// Every room a user has left.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_left<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = SyncStateEventItem> + Send + 'a {
        type KeyVal<'a> = ((Ignore, &'a str), Raw<Vec<Raw<AnySyncStateEvent>>>);

        let prefix = (user_id, Interfix);
        self.db
            .userroomid_leftstate
            .stream_prefix(&prefix)
            .ignore_err()
            .map(|((_, room_id), state): KeyVal<'_>| {
                let room_id = <&RoomId>::try_from(room_id).expect("valid room id in db");
                Ok((room_id.to_owned(), state.deserialize()?))
            })
            .ignore_err()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn invite_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_invitestate, user_id, room_id)
            .await
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn knock_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_knockedstate, user_id, room_id)
            .await
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn left_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_leftstate, user_id, room_id)
            .await
    }

    async fn stripped_state(
        &self,
        map: &Arc<Map>,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        let key = (user_id, room_id);
        map.qry(&key)
            .await
            .deserialized()
            .and_then(|val: Raw<Vec<Raw<AnyStrippedStateEvent>>>| {
                val.deserialize().map_err(Into::into)
            })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub async fn once_joined(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = once_joined_key(room_id, user_id);
        self.db.roomuseroncejoinedids.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_joined<'a>(&'a self, user_id: &'a UserId, room_id: &'a RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_joined.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_knocked<'a>(&'a self, user_id: &'a UserId, room_id: &'a RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_knockedstate.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_invited(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_invitestate.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_left(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_leftstate.qry(&key).await.is_ok()
    }

    /// A user's membership in a room, or `None` where they have none.
    ///
    /// A ban is a leave the indexes do not distinguish, so it is read off the
    /// one thing that does differ: a banned user has been joined at some point
    /// and now holds no membership at all.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn user_membership(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Option<MembershipState> {
        let states = join5(
            self.is_joined(user_id, room_id),
            self.is_left(user_id, room_id),
            self.is_knocked(user_id, room_id),
            self.is_invited(user_id, room_id),
            self.once_joined(user_id, room_id),
        )
        .await;

        match states {
            (true, ..) => Some(MembershipState::Join),
            (_, true, ..) => Some(MembershipState::Leave),
            (_, _, true, ..) => Some(MembershipState::Knock),
            (_, _, _, true, ..) => Some(MembershipState::Invite),
            (false, false, false, false, true) => Some(MembershipState::Ban),
            _ => None,
        }
    }

    /// The servers an invite named as able to serve the room, which is how a
    /// user joins a room this server has never otherwise heard of.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn servers_invite_via<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a ServerName> + Send + 'a {
        type KeyVal<'a> = (Ignore, Vec<&'a str>);

        // The whole list, not just its last entry: `add_servers_invite_via`
        // reads back through here to merge, so dropping any of them here would
        // lose them on the next update.
        self.db
            .roomid_inviteviaservers
            .stream_prefix(room_id)
            .ignore_err()
            .map(|(_, servers): KeyVal<'_>| servers)
            .flat_map(|servers| {
                servers
                    .into_iter()
                    .map(|server| <&ServerName>::try_from(server).expect("valid server name in db"))
                    .stream()
            })
    }

    /// Adds to the servers recorded by [`servers_invite_via`], keeping the
    /// list sorted and free of duplicates.
    ///
    /// [`servers_invite_via`]: Self::servers_invite_via
    #[tracing::instrument(level = "debug", skip(self, servers))]
    pub async fn add_servers_invite_via(&self, room_id: &RoomId, servers: Vec<OwnedServerName>) {
        let mut servers: Vec<_> = self
            .servers_invite_via(room_id)
            .map(ToOwned::to_owned)
            .chain(iter(servers))
            .collect()
            .await;

        servers.sort_unstable();
        servers.dedup();

        let servers = servers
            .iter()
            .map(|server| server.as_bytes())
            .collect::<Vec<_>>()
            .join(&[0xFF][..]);

        self.db
            .roomid_inviteviaservers
            .insert(room_id.as_bytes(), &servers)
            .ok();
    }

    /// Up to five servers likely to still be in the room some time from now,
    /// which is what a room's permalinks are built from.
    ///
    /// See <https://spec.matrix.org/latest/appendices/#routing>.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn servers_route_via(&self, room_id: &RoomId) -> Result<Vec<OwnedServerName>> {
        // The server of the most powerful user goes first: it is the one least
        // likely to lose its claim on the room.
        let most_powerful_user_server = self
            .services
            .state_accessor
            .room_state_get_content(room_id, &StateEventType::RoomPowerLevels, "")
            .await
            .map(|content: RoomPowerLevelsEventContent| {
                content
                    .users
                    .iter()
                    .max_by_key(|(_, power)| *power)
                    .and_then(|x| (x.1 >= &int!(50)).then_some(x))
                    .map(|(user, _power)| user.server_name().to_owned())
            });

        let mut counts: HashMap<OwnedServerName, usize> = HashMap::new();
        self.room_members(room_id)
            .ready_for_each(|user| {
                *counts.entry(user.server_name().to_owned()).or_default() += 1;
            })
            .await;

        let mut by_members: Vec<_> = counts.into_iter().collect();
        by_members.sort_unstable_by_key(|(_, users)| *users);

        let mut servers: Vec<OwnedServerName> = by_members
            .into_iter()
            .map(|(server, _)| server)
            .rev()
            .take(5)
            .collect();

        if let Ok(Some(server)) = most_powerful_user_server {
            servers.insert(0, server);
            servers.truncate(5);
        }

        Ok(servers)
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

        // There is no knocked count to match these two. The reference writes
        // one into `roomuserid_knockedcount`, but that column is keyed by
        // (room, user) and nothing reads a room-only key back out of it, so
        // the write is dead. A room-level knocked count wants a
        // `roomid_knockedcount` column of its own; until there is one,
        // `room_members_knocked(room_id).count()` is the answer.
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

                // Not in the room any more, in either direction.
                self.db.roomserverids.del((room_id, old_joined_server)).ok();
                self.db.serverroomids.del((old_joined_server, room_id)).ok();
            })
            .await;

        // Whatever is left in `joined_servers` was not already recorded.
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
