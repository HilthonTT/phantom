//! Shutting a room down.
//!
//! What an operator means by "close this room": every local user is put out of
//! it, its local aliases are freed, and it stops being advertised. The room is
//! still on disk afterwards, which is the point — an abuse report is
//! investigated against the history, not against the absence of it.
//!
//! A shutdown runs under the room's state mutex, which the caller takes and
//! holds across the whole thing, because evicting a user writes a membership
//! event like any other.
//!
//! Eviction sends each local user's own leave event, falling back to clearing
//! the membership indexes where that cannot be done, which is the same shape
//! as the account-teardown path in [`deactivate`]. It stops at this server's
//! users. Telling the remote members' servers that the room is gone is not a
//! thing the spec offers, and evicting them would need `make_leave` and
//! `send_leave` on their behalf, which belongs to the planned `membership`
//! service.
//!
//! [`deactivate`]: crate::deactivate

use std::sync::Arc;

use futures::StreamExt;
use phantom_core::{Result, debug, matrix::PduBuilder, result::LogErr, warn};
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
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    timeline: Dep<rooms::timeline::Service>,
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
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Evicts every local user, frees the room's local aliases, and
    /// unpublishes it from the directory.
    ///
    /// Reversible, and deliberately so: nothing here touches the timeline, and
    /// a local user's record of having left survives it, so a room that was
    /// shut down in error can be reopened.
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

    /// Puts one local user out of the room by sending their own leave event.
    ///
    /// Sent as the user rather than as the server, because a membership event
    /// is authorized against its state key: a leave the room will accept is
    /// one the leaving user sent. Where the room will not take it — they are
    /// not a member as far as its state is concerned, or the event is refused
    /// — the membership indexes are cleared locally instead, so the room stops
    /// appearing in their sync either way.
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
