use phantom_core::{Err, Result, implement, matrix::PduBuilder};
use ruma::{RoomId, UserId, events::room::member::MembershipState};

use super::Service;
use crate::rooms::state::RoomMutexGuard;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(%sender_user, %room_id, %user_id))]
pub async fn kick(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
    reason: Option<&String>,
    sender_user: &UserId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let Ok(mut content) = self
        .services
        .state_accessor
        .get_member(room_id, user_id)
        .await
    else {
        return Err!(Request(Forbidden(
            "Cannot kick a user who is not in the room."
        )));
    };

    if !matches!(
        content.membership,
        MembershipState::Invite | MembershipState::Knock | MembershipState::Join,
    ) {
        return Err!(Request(Forbidden(
            "Cannot kick a user who is not apart of the room (current membership: {})",
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
