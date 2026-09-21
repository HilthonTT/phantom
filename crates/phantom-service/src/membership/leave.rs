use std::collections::HashSet;

use futures::{FutureExt, StreamExt};
use phantom_core::{Err, Result, debug_info, debug_warn, err, implement, matrix::PduBuilder, warn};
use ruma::{
    CanonicalJsonObject, OwnedServerName, RoomId, UserId,
    api::{
        error::ErrorKind,
        federation::membership::{create_leave_event, prepare_leave_event},
    },
    canonical_json::to_canonical_value,
    events::{
        AnyStrippedStateEvent, StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
    serde::Raw,
};

use super::{Service, outgoing_pdu, sender_servers};
use crate::rooms::state::RoomMutexGuard;

#[implement(Service)]
#[tracing::instrument(name = "leave", level = "debug", skip_all, fields(%room_id, %user_id))]
pub async fn leave<'a>(
    &'a self,
    user_id: &'a UserId,
    room_id: &'a RoomId,
    reason: Option<String>,
    remote_leave_now: bool,
    state_lock: &'a RoomMutexGuard,
) -> Result {
    let mut leave_content = RoomMemberEventContent::new(MembershipState::Leave);
    leave_content.reason = reason.clone();

    let metadata = &self.services.metadata;

    if metadata.is_banned(room_id).await || metadata.is_disabled(room_id).await {
        return self
            .clear_local_leave(user_id, room_id, leave_content, None)
            .await;
    }

    let member_event = self.member_content(room_id, user_id).await;

    let state_cache = &self.services.state_cache;

    let dont_have_room = member_event.is_err()
        && !state_cache
            .server_in_room(self.services.server_state.server_name(), room_id)
            .await;

    let leave_remotely =
        remote_leave_now || (dont_have_room && !state_cache.is_knocked(user_id, room_id).await);

    if leave_remotely {
        self.leave_via_remote(user_id, room_id, reason, leave_content)
            .await
    } else {
        self.leave_locally(
            user_id,
            room_id,
            reason,
            leave_content,
            member_event,
            state_lock,
        )
        .await
    }
}

#[implement(Service)]
async fn member_content(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
) -> Result<RoomMemberEventContent> {
    self.services
        .state_accessor
        .room_state_get_content::<RoomMemberEventContent>(
            room_id,
            &StateEventType::RoomMember,
            user_id.as_str(),
        )
        .await
}

#[implement(Service)]
async fn leave_via_remote(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    leave_content: RoomMemberEventContent,
) -> Result {
    if let Err(e) = self.remote_leave(user_id, room_id, reason).boxed().await {
        warn!(%user_id, "Failed to leave room {room_id} remotely: {e}");
    }

    let last_state = self.last_known_strip_state(user_id, room_id).await;

    self.clear_local_leave(user_id, room_id, leave_content, last_state)
        .await
}

#[implement(Service)]
async fn last_known_strip_state(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
) -> Option<Vec<Raw<AnyStrippedStateEvent>>> {
    let state_cache = &self.services.state_cache;

    if let Ok(state) = state_cache.invite_state(user_id, room_id).await {
        return Some(state);
    }

    if let Ok(state) = state_cache.knock_state(user_id, room_id).await {
        return Some(state);
    }

    state_cache.left_state(user_id, room_id).await.ok()
}

#[implement(Service)]
async fn leave_locally(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    leave_content: RoomMemberEventContent,
    member_event: Result<RoomMemberEventContent>,
    state_lock: &RoomMutexGuard,
) -> Result {
    let Ok(mut content) = member_event else {
        debug_warn!(
            "Trying to leave a room you are not a member of, marking room as left locally."
        );

        return self
            .clear_local_leave(user_id, room_id, leave_content, None)
            .await;
    };

    if !is_leaveable(&content.membership) {
        debug_warn!(
            current = ?content.membership,
            "Room state shows non-leaveable membership; clearing local caches.",
        );

        return self
            .clear_local_leave(user_id, room_id, leave_content, None)
            .await;
    }

    content.membership = MembershipState::Leave;
    content.reason = reason;
    content.join_authorized_via_users_server = None;
    content.is_direct = None;

    let built = self
        .services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(user_id.to_string(), &content),
            user_id,
            room_id,
            state_lock,
        )
        .await;

    let error = match built {
        Ok(_) => return Ok(()),
        Err(error) if matches!(error.kind(), ErrorKind::Forbidden) => error,
        Err(error) => return Err(error),
    };

    let current = self
        .member_content(room_id, user_id)
        .await
        .map(|content| content.membership);

    if current.as_ref().is_ok_and(is_leaveable) {
        return Err(error);
    }

    warn!(%error, ?current, "Auth refused self-leave PDU; clearing local caches.");

    self.clear_local_leave(user_id, room_id, leave_content, None)
        .await
}

#[implement(Service)]
async fn clear_local_leave(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    leave_content: RoomMemberEventContent,
    last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
) -> Result {
    self.services
        .state_cache
        .update_membership(
            room_id,
            user_id,
            leave_content,
            user_id,
            last_state,
            None,
            true,
        )
        .await
}

#[implement(Service)]
async fn leave_servers(&self, user_id: &UserId, room_id: &RoomId) -> HashSet<OwnedServerName> {
    let state_cache = &self.services.state_cache;
    let server_state = &self.services.server_state;

    let mut servers: HashSet<OwnedServerName> = state_cache
        .servers_invite_via(room_id)
        .chain(state_cache.room_servers(room_id))
        .map(ToOwned::to_owned)
        .collect()
        .await;

    if let Ok(invite_state) = state_cache.invite_state(user_id, room_id).await {
        servers.extend(sender_servers(&invite_state));
    } else if let Ok(knock_state) = state_cache.knock_state(user_id, room_id).await {
        servers.extend(sender_servers(&knock_state));
    }

    servers.insert(user_id.server_name().to_owned());

    if let Some(room_server) = room_id.server_name() {
        servers.insert(room_server.to_owned());
    }

    servers.retain(|server| !server_state.server_is_ours(server));

    servers
}

#[implement(Service)]
#[tracing::instrument(name = "remote", level = "debug", skip_all)]
async fn remote_leave(&self, user_id: &UserId, room_id: &RoomId, reason: Option<String>) -> Result {
    let servers = self.leave_servers(user_id, room_id).await;

    debug_info!("servers in remote_leave_room: {servers:?}");

    let mut make_leave = Err!(BadServerResponse(
        "No remote server available to assist in leaving {room_id}."
    ));

    for remote_server in servers {
        let request = prepare_leave_event::v1::Request::new(room_id.to_owned(), user_id.to_owned());

        make_leave = self
            .services
            .federation
            .execute(&remote_server, request)
            .await
            .map(|response| (response, remote_server));

        if make_leave.is_ok() {
            break;
        }
    }

    let (make_leave_response, remote_server) = make_leave?;

    let Some(room_version_id) = make_leave_response.room_version else {
        return Err!(BadServerResponse(warn!(
            "No room version was returned by {remote_server} for {room_id}, room version is \
             likely not supported by phantom"
        )));
    };

    if !self.supported_room_version(&room_version_id) {
        return Err!(BadServerResponse(warn!(
            "Remote room version {room_version_id} for {room_id} is not supported by phantom"
        )));
    }

    let mut event: CanonicalJsonObject = serde_json::from_str(make_leave_response.event.get())
        .map_err(|e| {
            err!(BadServerResponse(warn!(
                "Invalid make_leave event json received from {remote_server} for {room_id}: \
                 {e:?}"
            )))
        })?;

    let mut content = RoomMemberEventContent::new(MembershipState::Leave);
    content.reason = reason;

    self.services
        .profile
        .fill_profile_data(user_id, &mut content)
        .await;

    self.complete_member_event(&mut event, room_id, user_id, to_canonical_value(content)?)?;

    let event_id = self
        .services
        .server_keys
        .gen_id_hash_and_sign_event(&mut event, &room_version_id)?;

    let request = create_leave_event::v2::Request::new(
        room_id.to_owned(),
        event_id,
        outgoing_pdu(event, &room_version_id),
    );

    self.services
        .federation
        .execute(&remote_server, request)
        .await?;

    Ok(())
}

fn is_leaveable(state: &MembershipState) -> bool {
    matches!(
        state,
        MembershipState::Invite | MembershipState::Join | MembershipState::Knock
    )
}

#[cfg(test)]
mod tests {
    use ruma::events::room::member::MembershipState;

    use super::is_leaveable;

    #[test]
    fn only_a_live_membership_can_be_left() {
        assert!(is_leaveable(&MembershipState::Invite));
        assert!(is_leaveable(&MembershipState::Join));
        assert!(is_leaveable(&MembershipState::Knock));

        assert!(!is_leaveable(&MembershipState::Leave));
        assert!(!is_leaveable(&MembershipState::Ban));
    }
}
