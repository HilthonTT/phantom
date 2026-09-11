//! Shutting a room down, and wiping what is left of it.
//!
//! Two phases, and a caller may want only the first. [`shutdown_room`] is what
//! an operator means by "close this room": every local user is put out of it,
//! its local aliases are freed, and it stops being advertised. The room is
//! still on disk afterwards, which is the point — an abuse report is
//! investigated against the history, not against the absence of it.
//!
//! [`purge_room`] is the second phase, and it is not reversible. Everything
//! this server holds for the room goes: the PDUs, the indexes built over them,
//! the membership records, the receipts. What it deliberately leaves behind is
//! anything shared with rooms that are staying — the compressed state blocks,
//! and the short ids assigned to events — because those are reclaimed by
//! sweeping what nothing points at any more, not by deleting one room.
//!
//! Both phases run under the room's state mutex, which the caller takes and
//! holds across the whole thing. Purging with it released would race an event
//! arriving over federation: the room's rows would go, and the event would
//! write a fresh set of them straight afterwards.
//!
//! Eviction here sends each local user's own leave event, falling back to
//! clearing the membership indexes where that cannot be done, which is the
//! same shape as the account-teardown path in [`deactivate`]. It stops at
//! this server's users. Telling the remote members' servers that the room is
//! gone is not a thing the spec offers, and evicting them would need
//! `make_leave`/`send_leave` on their behalf, which belongs to the planned
//! `membership` service. The same goes for tuwunel's `delete_if_empty_local`,
//! the auto-delete hook that fires when the last local user leaves: it is a
//! caller of this service rather than part of it, and the leave path that
//! would call it does not exist yet.
//!
//! [`deactivate`]: crate::deactivate
//! [`purge_room`]: Service::purge_room
//! [`shutdown_room`]: Service::shutdown_room

use std::sync::Arc;

use futures::StreamExt;
use phantom_core::{Result, debug, debug_info, matrix::PduBuilder, result::LogErr, warn};
use ruma::{
    OwnedRoomAliasId, OwnedRoomId, OwnedUserId, RoomId, UserId,
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};
use serde::{Deserialize, Serialize};

use crate::{Dep, rooms, rooms::state::RoomMutexGuard};

/// What a leaving user is told the room was closed for.
const LEAVE_REASON: &str = "Room deleted";

pub struct Service {
    services: Services,
}

struct Services {
    alias: Dep<rooms::alias::Service>,
    directory: Dep<rooms::directory::Service>,
    pdu_metadata: Dep<rooms::pdu_metadata::Service>,
    read_receipt: Dep<rooms::read_receipt::Service>,
    search: Dep<rooms::search::Service>,
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    threads: Dep<rooms::threads::Service>,
    timeline: Dep<rooms::timeline::Service>,
    user: Dep<rooms::user::Service>,
}

/// Records local-user eviction results and aliases targeted for removal.
///
/// Its serialized layout matches Synapse's `ShutdownRoom`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShutdownRoom {
    pub kicked_users: Vec<OwnedUserId>,
    pub failed_to_kick_users: Vec<OwnedUserId>,
    pub local_aliases: Vec<OwnedRoomAliasId>,
    pub new_room_id: Option<OwnedRoomId>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            services: Services {
                alias: args.depend::<rooms::alias::Service>("rooms::alias"),
                directory: args.depend::<rooms::directory::Service>("rooms::directory"),
                pdu_metadata: args.depend::<rooms::pdu_metadata::Service>("rooms::pdu_metadata"),
                read_receipt: args.depend::<rooms::read_receipt::Service>("rooms::read_receipt"),
                search: args.depend::<rooms::search::Service>("rooms::search"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                threads: args.depend::<rooms::threads::Service>("rooms::threads"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
                user: args.depend::<rooms::user::Service>("rooms::user"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Shuts the room down and then wipes it, returning what the shutdown
    /// found.
    ///
    /// `force` widens the erasure of local users' left-state; see
    /// [`purge_room`](Self::purge_room).
    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    pub async fn delete_room(
        &self,
        room_id: &RoomId,
        force: bool,
        state_lock: &RoomMutexGuard,
    ) -> Result<ShutdownRoom> {
        let summary = self.shutdown_room(room_id, state_lock).await;

        self.purge_room(room_id, force, state_lock).await;

        debug_info!(%room_id, "Deleted room");

        Ok(summary)
    }

    /// Evicts every local user, frees the room's local aliases, and
    /// unpublishes it from the directory.
    ///
    /// The reversible half of a delete, and the whole of it when an operator
    /// wants the room closed but kept. Nothing here touches the timeline: the
    /// room's history survives a shutdown, and so does a local user's record
    /// of having left it.
    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    pub async fn shutdown_room(
        &self,
        room_id: &RoomId,
        state_lock: &RoomMutexGuard,
    ) -> ShutdownRoom {
        debug!(%room_id, "Evicting local users");

        // Collected before the first eviction rather than streamed: each one
        // writes a membership event, which is a write to the very column the
        // stream would be reading.
        let local_users: Vec<OwnedUserId> = self
            .services
            .state_cache
            .local_users_in_room(room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        let mut kicked_users = Vec::new();
        let mut failed_to_kick_users = Vec::new();

        for user_id in local_users {
            match self.evict(&user_id, room_id, state_lock).await {
                Ok(()) => kicked_users.push(user_id),
                Err(e) => {
                    warn!(%user_id, %room_id, "Failed to evict user from room: {e}");
                    failed_to_kick_users.push(user_id);
                }
            }
        }

        debug!(%room_id, "Freeing the room's local aliases");

        let local_aliases: Vec<OwnedRoomAliasId> = self
            .services
            .alias
            .local_aliases_for_room(room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        for alias in &local_aliases {
            self.services.alias.remove_alias(alias).await.log_err().ok();
        }

        debug!(%room_id, "Unpublishing the room from the directory");
        self.services
            .directory
            .set_not_public(room_id)
            .log_err()
            .ok();

        ShutdownRoom {
            kicked_users,
            failed_to_kick_users,
            local_aliases,
            // Set by the caller where the shutdown is part of a room upgrade
            // and the members are being pointed at a replacement.
            new_room_id: None,
        }
    }

    /// Wipes everything this server holds for the room.
    ///
    /// `force` decides what happens to the local users who have left: their
    /// leave record is what shows the room in the `leave` section of a sync,
    /// which is how a client learns the room ended rather than finding it
    /// silently gone, so it is kept unless `force`.
    ///
    /// Failures are logged and stepped over. A column that will not give up
    /// its rows leaves a fragment of a room behind; stopping there would leave
    /// the rest of it too.
    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    async fn purge_room(&self, room_id: &RoomId, force: bool, state_lock: &RoomMutexGuard) {
        // Everything keyed by short room id has to be reached before the room
        // gives that id up, which is why it is resolved once here and passed
        // down rather than looked up per column.
        let Ok(shortroomid) = self.services.short.get_shortroomid(room_id).await else {
            debug!(%room_id, "Room has no short id, so there is nothing stored to purge");
            return;
        };

        // The unread counters are keyed by user, so they can only be found
        // for users that are named. Read before the membership indexes go.
        let members: Vec<OwnedUserId> = self
            .services
            .state_cache
            .room_useroncejoined(room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        debug!(%room_id, "Deleting the room's events");
        match self
            .services
            .timeline
            .delete_all_pdus(room_id, shortroomid)
            .await
        {
            Ok(deleted) => debug!(%room_id, deleted, "Deleted the room's events"),
            Err(e) => warn!(%room_id, "Failed to delete the room's events: {e}"),
        }

        debug!(%room_id, "Deleting the room's threads");
        self.services.threads.delete_all_threads(shortroomid).await;

        debug!(%room_id, "Deleting the room's search index");
        self.services.search.delete_all_tokenids(shortroomid).await;

        debug!(%room_id, "Deleting the room's forward extremities");
        self.services
            .state
            .delete_all_forward_extremities(room_id, state_lock)
            .await;

        debug!(%room_id, "Deleting the room's event references");
        self.services
            .pdu_metadata
            .delete_all_referenced(room_id)
            .await;

        debug!(%room_id, "Deleting the room's read receipts");
        self.services
            .read_receipt
            .delete_all_read_receipts(room_id)
            .await;

        debug!(%room_id, "Deleting the room's notification state");
        self.services
            .user
            .delete_room_notification_state(room_id, shortroomid, &members)
            .await;

        debug!(%room_id, "Deleting the room's membership indexes");
        self.services
            .state_cache
            .delete_room_memberships(room_id, force)
            .await;

        debug!(%room_id, "Deleting the room's state version");
        self.services
            .state
            .delete_room_shortstatehash(room_id, state_lock)
            .log_err()
            .ok();

        debug!(%room_id, "Deleting the room's short id");
        self.services
            .short
            .delete_shortroomid(room_id)
            .log_err()
            .ok();
    }

    /// Puts one local user out of the room by sending their own leave event.
    ///
    /// Sent as the user rather than as the server, because a membership event
    /// is authorized against its state key: a leave the room will accept is
    /// one the leaving user sent. Where the room will not take it — they are
    /// not a member as far as its state is concerned, or the event is refused
    /// — the membership indexes are cleared locally instead, so the room
    /// stops appearing in their sync either way.
    async fn evict(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        state_lock: &RoomMutexGuard,
    ) -> Result {
        let member_event = self
            .services
            .state_accessor
            .room_state_get_content::<RoomMemberEventContent>(
                room_id,
                &StateEventType::RoomMember,
                user_id.as_str(),
            )
            .await;

        let leavable = member_event.as_ref().is_ok_and(|event| {
            matches!(
                event.membership,
                MembershipState::Invite | MembershipState::Join | MembershipState::Knock
            )
        });

        if !leavable {
            return self.clear_local_leave(user_id, room_id).await;
        }

        let mut content = member_event.expect("checked just above");
        content.membership = MembershipState::Leave;
        content.reason = Some(LEAVE_REASON.to_owned());
        content.join_authorized_via_users_server = None;
        content.is_direct = None;

        let sent = self
            .services
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(user_id.to_string(), &content),
                user_id,
                room_id,
                state_lock,
            )
            .await;

        match sent {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!(%user_id, %room_id, "Failed to send the user's leave event: {e}");

                self.clear_local_leave(user_id, room_id).await
            }
        }
    }

    /// Records the user as having left, in this server's indexes alone.
    ///
    /// The room's other members still see the membership as it was. That is
    /// the right trade for a room being closed: the local user's client stops
    /// showing it, and no half-authorized event is forced into a room that
    /// refused one.
    async fn clear_local_leave(&self, user_id: &UserId, room_id: &RoomId) -> Result {
        let leave_content = RoomMemberEventContent::new(MembershipState::Leave);

        self.services
            .state_cache
            .update_membership(room_id, user_id, leave_content, user_id, None, None, true)
            .await
    }
}

#[cfg(test)]
mod tests {
    use phantom_core::matrix::pdu::{PduCount, PduId, RawPduId};
    use phantom_database::{SEP, serialize_to_vec};

    use crate::rooms::short::ShortRoomId;

    /// Four of the columns a purge clears are wiped by short room id alone —
    /// the timeline, the thread participants, the search index and the sync
    /// tokens — and each one builds its keys itself, without going through a
    /// shared constructor. All four hold only because a bare `u64` serializes
    /// to the same eight big-endian bytes those keys open with. Nothing else
    /// checks that: get it wrong and the prefix matches nothing, so the purge
    /// reports success over columns it never touched.
    #[test]
    fn a_short_room_id_prefixes_every_key_purged_by_it() {
        const SHORTROOMID: ShortRoomId = 0x0123_4567_89ab_cdef;

        let prefix = serialize_to_vec(SHORTROOMID).expect("serialized");
        assert_eq!(
            prefix,
            SHORTROOMID.to_be_bytes(),
            "a short room id must serialize to its big-endian bytes",
        );

        let pdu_id: RawPduId = PduId {
            shortroomid: SHORTROOMID,
            shorteventid: PduCount::Normal(7),
        }
        .into();

        assert!(
            pdu_id.as_ref().starts_with(&prefix),
            "a pdu id must be reachable from its room's prefix",
        );

        // As `rooms::search` builds one: the room, the word, and the pdu the
        // word was found in.
        let mut token_id = Vec::from(SHORTROOMID.to_be_bytes());
        token_id.extend_from_slice(b"word");
        token_id.push(SEP);
        token_id.extend_from_slice(pdu_id.as_ref());

        assert!(
            token_id.starts_with(&prefix),
            "a search token must be reachable from its room's prefix",
        );

        // As `rooms::user` builds one: the room, and the sync token held for
        // it.
        let sync_token: &[u64] = &[SHORTROOMID, 42];
        let sync_token = serialize_to_vec(sync_token).expect("serialized");

        assert!(
            sync_token.starts_with(&prefix),
            "a room sync token must be reachable from its room's prefix",
        );
    }
}
