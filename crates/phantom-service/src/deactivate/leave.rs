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
