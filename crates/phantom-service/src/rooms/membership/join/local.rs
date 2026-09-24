//! The local join, and the restricted-room fallback to federation.

use futures::{StreamExt, stream};
use phantom_core::{
    Err, Result, debug_info, err, implement,
    matrix::{PduBuilder, pdu::gen_event_id_canonical_json},
    warn,
};
use ruma::{
    CanonicalJsonObject, OwnedServerName, OwnedUserId, RoomId, UserId,
    events::{
        StateEventType,
        room::{
            join_rules::RoomJoinRulesEventContent,
            member::{MembershipState, RoomMemberEventContent},
        },
    },
    room::{AllowRule, JoinRule},
};
use serde_json::value::to_raw_value;

use crate::{
    rooms::membership::{Service, merge_member_content},
    rooms::state::RoomMutexGuard,
};

#[implement(Service)]
#[tracing::instrument(name = "local", level = "debug", skip_all)]
pub(super) async fn join_local(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    state_lock: RoomMutexGuard,
    extra_content: Option<CanonicalJsonObject>,
) -> Result {
    debug_info!("We can join locally");

    let restriction_rooms = self.restriction_rooms(room_id).await;

    let is_joined_restricted_rooms = stream::iter(&restriction_rooms)
        .any(|restriction_room_id| {
            self.services
                .state_cache
                .is_joined(sender_user, restriction_room_id)
        })
        .await;

    let join_authorized_via_users_server = if is_joined_restricted_rooms {
        self.authorizing_user(room_id, sender_user, &state_lock)
            .await
    } else {
        None
    };

    let mut content = RoomMemberEventContent::new(MembershipState::Join);
    content.reason = reason.clone();
    content.join_authorized_via_users_server = join_authorized_via_users_server;

    self.services
        .profile
        .fill_profile_data(sender_user, &mut content)
        .await;

    let content = merge_member_content(content, extra_content.as_ref())?;

    let pdu_builder = PduBuilder {
        event_type: StateEventType::RoomMember.into(),
        content: to_raw_value(&content)?,
        state_key: Some(sender_user.as_str().into()),
        ..PduBuilder::default()
    };

    let Err(error) = self
        .services
        .timeline
        .build_and_append_pdu(pdu_builder, sender_user, room_id, &state_lock)
        .await
    else {
        return Ok(());
    };

    if restriction_rooms.is_empty() && self.is_local_only(servers) {
        return Err(error);
    }

    warn!(
        "We couldn't do the join locally, maybe federation can help to satisfy the restricted \
         join requirements"
    );

    drop(state_lock);

    let Ok((make_join_response, remote_server)) =
        self.make_join_request(sender_user, room_id, servers).await
    else {
        return Err(error);
    };

    let room_version_id = self.require_supported_remote_room_version(&make_join_response)?;

    let (join_event, event_id, _) = self
        .create_join_event(
            room_id,
            sender_user,
            &make_join_response.event,
            &room_version_id,
            reason,
            extra_content,
        )
        .await?;

    let send_join_response = self
        .execute_send_join(
            &remote_server,
            room_id,
            &event_id,
            join_event,
            &room_version_id,
        )
        .await?;

    let Some(signed_raw) = send_join_response.event else {
        return Err(error);
    };

    let (signed_event_id, signed_value) =
        gen_event_id_canonical_json(&signed_raw, &room_version_id).map_err(|e| {
            err!(Request(BadJson(warn!(
                "Could not convert event to canonical JSON: {e}"
            ))))
        })?;

    if signed_event_id != event_id {
        return Err!(Request(BadJson(warn!(
            "Server {remote_server} sent event {signed_event_id} when {event_id} was expected"
        ))));
    }

    self.services
        .event_handler
        .handle_incoming_pdu(
            &remote_server,
            room_id,
            &signed_event_id,
            signed_value,
            true,
        )
        .await?
        .ok_or_else(|| {
            err!(Request(InvalidParam(
                "Signed join was not accepted as a timeline event."
            )))
        })?;

    Ok(())
}

#[implement(Service)]
async fn restriction_rooms(&self, room_id: &RoomId) -> Vec<ruma::OwnedRoomId> {
    let Ok(join_rules) = self
        .services
        .state_accessor
        .room_state_get_content::<RoomJoinRulesEventContent>(
            room_id,
            &StateEventType::RoomJoinRules,
            "",
        )
        .await
    else {
        return Vec::new();
    };

    match join_rules.join_rule {
        JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted) => restricted
            .allow
            .into_iter()
            .filter_map(|rule| match rule {
                AllowRule::RoomMembership(membership) => Some(membership.room_id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[implement(Service)]
async fn authorizing_user(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    state_lock: &RoomMutexGuard,
) -> Option<OwnedUserId> {
    let local_users: Vec<OwnedUserId> = self
        .services
        .state_cache
        .local_users_in_room(room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for user in local_users {
        if self
            .services
            .state_accessor
            .user_can_invite(room_id, &user, sender_user, state_lock)
            .await
        {
            return Some(user);
        }
    }

    None
}
