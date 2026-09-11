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

    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    pub async fn shutdown_room(
        &self,
        room_id: &RoomId,
        state_lock: &RoomMutexGuard,
    ) -> ShutdownRoom {
        debug!(%room_id, "Evicting local users");

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

            new_room_id: None,
        }
    }

    #[tracing::instrument(skip(self, state_lock), level = "debug")]
    async fn purge_room(&self, room_id: &RoomId, force: bool, state_lock: &RoomMutexGuard) {
        let Ok(shortroomid) = self.services.short.get_shortroomid(room_id).await else {
            debug!(%room_id, "Room has no short id, so there is nothing stored to purge");
            return;
        };

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

        let mut token_id = Vec::from(SHORTROOMID.to_be_bytes());
        token_id.extend_from_slice(b"word");
        token_id.push(SEP);
        token_id.extend_from_slice(pdu_id.as_ref());

        assert!(
            token_id.starts_with(&prefix),
            "a search token must be reachable from its room's prefix",
        );

        let sync_token: &[u64] = &[SHORTROOMID, 42];
        let sync_token = serialize_to_vec(sync_token).expect("serialized");

        assert!(
            sync_token.starts_with(&prefix),
            "a room sync token must be reachable from its room's prefix",
        );
    }
}
