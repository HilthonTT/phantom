use phantom_core::{Result, implement, matrix::PduBuilder};
use ruma::{
    RoomId, UserId,
    events::room::member::{MembershipState, RoomMemberEventContent},
};

use super::Service;
use crate::rooms::state::RoomMutexGuard;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(%sender_user, %room_id, %user_id))]
pub async fn ban(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
    reason: Option<&String>,
    sender_user: &UserId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let mut content = RoomMemberEventContent::new(MembershipState::Ban);
    content.reason = reason.cloned();

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
