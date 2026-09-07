//! Leaving the rooms a deactivating account is still in.
//!
//! Two steps, in this order and for this reason: a user who holds power in a
//! room gives it up first, because once they have left they no longer have
//! the power level to give anything up, and a room whose only admin walks out
//! without demoting themselves is left with an admin who is gone.
//!
//! Leaving is a membership event like any other, so it goes through the room's
//! state mutex and the timeline write path, and the room's other members learn
//! of it the way they learn of anything else. Where that cannot be done — the
//! user is not a member as far as the room's state is concerned, or the event
//! is refused — the membership indexes are cleared locally anyway, so the room
//! stops appearing in the user's sync. The account is being torn down, and one
//! room that will not let go of it is not a reason to leave the rest half
//! done.
//!
//! Leaving a room this server is not in at all is the one case not handled
//! here. It needs `make_leave`/`send_leave` over federation, which belongs to
//! the planned `membership` service rather than to this one.

use futures::StreamExt;
use phantom_core::{
    Result, err, implement, info,
    matrix::{Event, PduBuilder},
    warn,
};
use ruma::{
    OwnedRoomId, RoomId, UserId,
    events::{
        StateEventType,
        room::{
            member::{MembershipState, RoomMemberEventContent},
            power_levels::RoomPowerLevelsEventContent,
        },
    },
};

/// Gives up the user's own power level in every room they are joined to.
///
/// Only where they are allowed to: a user who cannot change their own level
/// cannot give it up either. The room's creator is let through on the strength
/// of the create event rather than the levels, since from room version 12 a
/// creator holds a level the power levels event never states.
///
/// Failures are logged and stepped over. A room that will not take the
/// demotion is not a reason to abandon the deactivation.
#[implement(super::Service)]
pub(super) async fn demote_self(&self, user_id: &UserId) -> Result {
    let all_joined_rooms: Vec<OwnedRoomId> = self
        .services
        .state_cache
        .rooms_joined(user_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for room_id in all_joined_rooms {
        let state_lock = self.services.state.mutex.lock(&*room_id).await;

        let room_power_levels = self
            .services
            .state_accessor
            .get_power_levels(&room_id)
            .await
            .ok();

        let user_can_change_self = room_power_levels.as_ref().is_some_and(|power_levels| {
            power_levels.user_can_change_user_power_level(user_id, user_id)
        });

        let user_can_demote_self = user_can_change_self
            || self
                .services
                .state_accessor
                .room_state_get(&room_id, &StateEventType::RoomCreate, "")
                .await
                .is_ok_and(|event| event.sender() == user_id);

        if !user_can_demote_self {
            continue;
        }

        let mut power_levels_content: RoomPowerLevelsEventContent = match room_power_levels {
            Some(power_levels) => power_levels
                .try_into()
                .map_err(|e| err!(Database("Invalid power levels in {room_id}: {e}")))?,
            None => {
                let (rules, _creators) = self
                    .services
                    .state_accessor
                    .power_level_context(&room_id)
                    .await;

                RoomPowerLevelsEventContent::new(&rules)
            }
        };

        power_levels_content.users.remove(user_id);

        match self
            .services
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(String::new(), &power_levels_content),
                user_id,
                &room_id,
                &state_lock,
            )
            .await
        {
            Err(e) => {
                warn!(%room_id, %user_id, "Failed to demote user's own power level: {e}");
            }
            _ => {
                info!("Demoted {user_id} in {room_id} as part of account deactivation");
            }
        }
    }

    Ok(())
}

/// Leaves `room_id` by sending the user's own `m.room.member` leave event,
/// which is what tells the room's other members the account is gone.
///
/// Falls back to [`clear_local_leave`](Self::clear_local_leave) where the
/// event cannot be built or sent. Either way the room stops appearing in the
/// user's sync, which is what deactivation has to guarantee.
#[implement(super::Service)]
pub(super) async fn leave_room(&self, user_id: &UserId, room_id: &RoomId) {
    let state_lock = self.services.state.mutex.lock(room_id).await;

    let member_event = self
        .services
        .state_accessor
        .room_state_get_content::<RoomMemberEventContent>(
            room_id,
            &StateEventType::RoomMember,
            user_id.as_str(),
        )
        .await;

    // Not a member as far as the room's state is concerned, or in a state
    // there is no leaving from. Either way there is no event to send.
    let Ok(event) = member_event else {
        return self.clear_local_leave(user_id, room_id).await;
    };

    if !matches!(
        event.membership,
        MembershipState::Invite | MembershipState::Join | MembershipState::Knock
    ) {
        return self.clear_local_leave(user_id, room_id).await;
    }

    let mut content = event;
    content.membership = MembershipState::Leave;
    content.reason = None;
    content.join_authorized_via_users_server = None;
    content.is_direct = None;

    if let Err(e) = self
        .services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(user_id.to_string(), &content),
            user_id,
            room_id,
            &state_lock,
        )
        .await
    {
        warn!(%user_id, %room_id, "Failed to send the user's leave event: {e}");

        self.clear_local_leave(user_id, room_id).await;
    }
}

/// Records the user as having left, in this server's indexes alone.
///
/// The fallback for a room the leave event could not be sent to: the room
/// leaves the user's sync, and the room's other members still see the account
/// as it was.
#[implement(super::Service)]
async fn clear_local_leave(&self, user_id: &UserId, room_id: &RoomId) {
    let leave_content = RoomMemberEventContent::new(MembershipState::Leave);

    if let Err(e) = self
        .services
        .state_cache
        .update_membership(room_id, user_id, leave_content, user_id, None, None, true)
        .await
    {
        warn!(%user_id, %room_id, "Failed to record the user as having left: {e}");
    }
}
