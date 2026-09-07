//! The parts of deactivation that append events, waiting on the write path
//! they append through.
//!
//! This file is deliberately not a module of `deactivate`: nothing declares
//! `mod pending_write_path;`, so it is not compiled. It follows the same
//! convention as `rooms/timeline/pending_write_path.rs` and
//! `admin/pending_write_path.rs`, and for the same reason — this is the
//! reference implementation's code kept close to as-pasted so it can be
//! ported function by function as its dependencies land, rather than
//! rewritten from memory later.
//!
//! Both functions here call
//! [`rooms::timeline::build_and_append_pdu`](crate::rooms::timeline), which is
//! itself still parked in that service's `pending_write_path.rs`. When it
//! lands, these move into [`mod.rs`](super) and
//! [`full_deactivate`](super::Service::full_deactivate) grows the two calls
//! noted below:
//!
//! - `demote_self` runs over the joined rooms before any of them is left, so
//!   that a room whose only admin is deactivating is not left with an admin
//!   who is gone. It has to come first: once the user has left, they no
//!   longer have the power level to give it up.
//! - `leave_room` replaces the local-only
//!   [`leave_room`](super::Service::leave_room), whose doc comment says what
//!   is missing without it. The local half stays as the fallback for a room
//!   the event cannot be sent to.
//!
//! Leaving a room this server is not in at all is a third piece, and is not
//! here: it needs `make_leave`/`send_leave` over federation, which is the
//! planned `membership` service's to own rather than this one's.

use futures::StreamExt;
use phantom_core::{
    Result, implement, info,
    matrix::{Event, PduBuilder},
    warn,
};
use ruma::{
    OwnedRoomId, RoomId, UserId,
    events::{
        StateEventType,
        room::{member::RoomMemberEventContent, power_levels::RoomPowerLevelsEventContent},
    },
};

/// Gives up the user's own power level in every room they are joined to.
///
/// Only where they are allowed to: a user who cannot change their own level
/// cannot give it up either, and the room's creator is let through on the
/// strength of the create event rather than the levels, since from room
/// version 12 a creator holds a level the power levels event never states.
#[implement(super::Service)]
async fn demote_self(&self, user_id: &UserId) -> Result {
    let all_joined_rooms: Vec<OwnedRoomId> = self
        .services
        .state_cache
        .rooms_joined(user_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for room_id in all_joined_rooms {
        let state_lock = self.services.state.mutex.lock(&room_id).await;

        let room_power_levels = self
            .services
            .state_accessor
            .get_power_levels(&room_id)
            .await
            .ok();

        let user_can_change_self = room_power_levels
            .as_ref()
            .is_some_and(|power_levels| {
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

        let mut power_levels_content: RoomPowerLevelsEventContent = room_power_levels
            .map(TryInto::try_into)
            .transpose()?
            .unwrap_or_default();

        power_levels_content.users.remove(user_id);

        // ignore errors so deactivation doesn't fail
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
/// Falls back to the local half where the event cannot be built or sent —
/// the user is not a member as far as the room's state is concerned, or the
/// state they are in is one there is no leaving. Either way the room stops
/// appearing in their sync, which is what deactivation has to guarantee.
#[implement(super::Service)]
async fn leave_room(&self, user_id: &UserId, room_id: &RoomId) {
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

    let Ok(event) = member_event else {
        return self.clear_local_leave(user_id, room_id).await;
    };

    if !matches!(event.membership, Invite | Join | Knock) {
        return self.clear_local_leave(user_id, room_id).await;
    }

    let content = RoomMemberEventContent {
        membership: MembershipState::Leave,
        reason: None,
        join_authorized_via_users_server: None,
        is_direct: None,
        ..event
    };

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
