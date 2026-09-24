use phantom_core::{Err, Result, implement, matrix::PduBuilder};
use ruma::{
    RoomId, UserId,
    events::room::member::{MembershipState, RoomMemberEventContent},
};

use super::Service;
use crate::rooms::state::RoomMutexGuard;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(%sender_user, %room_id, %user_id))]
pub async fn unban(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
    reason: Option<&String>,
    sender_user: &UserId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let mut content = self
        .services
        .state_accessor
        .get_member(room_id, user_id)
        .await
        .unwrap_or_else(|_| RoomMemberEventContent::new(MembershipState::Leave));

    if content.membership != MembershipState::Ban {
        return Err!(Request(Forbidden(
            "Cannot unban a user who is not banned (current membership: {})",
            content.membership
        )));
    }

    content.membership = MembershipState::Leave;
    content.reason = reason.cloned();
    content.join_authorized_via_users_server = None;
    content.third_party_invite = None;
    content.is_direct = None;

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(user_id.to_string(), &content),
            sender_user,
            room_id,
            state_lock,
        )
        .await?;

    Ok(())
}
