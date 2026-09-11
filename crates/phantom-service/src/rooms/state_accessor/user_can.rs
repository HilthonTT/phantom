use phantom_core::{Err, Result, error, implement};
use ruma::{
    EventId, OwnedUserId, RoomId, UserId,
    events::{
        StateEventType, TimelineEventType,
        room::{
            create::RoomCreateEventContent,
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent, RoomPowerLevelsSource},
        },
    },
    room_version_rules::AuthorizationRules,
};

#[implement(super::Service)]
pub async fn user_can_redact(
    &self,
    redacts: &EventId,
    sender: &UserId,
    room_id: &RoomId,
    federation: bool,
) -> Result<bool> {
    let redacting_event = self.services.timeline.get_pdu(redacts).await;

    if redacting_event
        .as_ref()
        .is_ok_and(|pdu| pdu.kind == TimelineEventType::RoomCreate)
    {
        return Err!(Request(Forbidden(
            "Redacting m.room.create is not safe, forbidding."
        )));
    }

    if redacting_event
        .as_ref()
        .is_ok_and(|pdu| pdu.kind == TimelineEventType::RoomServerAcl)
    {
        return Err!(Request(Forbidden(
            "Redacting m.room.server_acl will result in the room being inaccessible for \
             everyone (empty allow key), forbidding."
        )));
    }

    match self
        .room_state_get_content::<RoomPowerLevelsEventContent>(
            room_id,
            &StateEventType::RoomPowerLevels,
            "",
        )
        .await
    {
        Ok(pl_event_content) => {
            let (rules, creators) = self.power_level_context(room_id).await;
            let pl_event = RoomPowerLevels::new(
                RoomPowerLevelsSource::Original(pl_event_content),
                &rules,
                creators,
            );

            Ok(pl_event.user_can_redact_event_of_other(sender)
                || pl_event.user_can_redact_own_event(sender)
                    && match redacting_event {
                        Ok(redacting_event) => {
                            if federation {
                                redacting_event.sender.server_name() == sender.server_name()
                            } else {
                                redacting_event.sender == sender
                            }
                        }
                        _ => false,
                    })
        }
        _ => match self
            .room_state_get(room_id, &StateEventType::RoomCreate, "")
            .await
        {
            Ok(room_create) => Ok(room_create.sender == sender
                || redacting_event
                    .as_ref()
                    .is_ok_and(|redacting_event| redacting_event.sender == sender)),
            _ => Err!(Database(
                "No m.room.power_levels or m.room.create events in database for room"
            )),
        },
    }
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn user_can_see_event(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
) -> bool {
    let Ok(shortstatehash) = self.pdu_shortstatehash(event_id).await else {
        return true;
    };

    let currently_member = self.services.state_cache.is_joined(user_id, room_id).await;

    let history_visibility = self
        .state_get_content(shortstatehash, &StateEventType::RoomHistoryVisibility, "")
        .await
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        );

    match history_visibility {
        HistoryVisibility::Invited => self.user_was_invited(shortstatehash, user_id).await,
        HistoryVisibility::Joined => self.user_was_joined(shortstatehash, user_id).await,
        HistoryVisibility::WorldReadable => true,
        HistoryVisibility::Shared => currently_member,
        _ => {
            error!(
                %room_id,
                %user_id,
                ?history_visibility,
                "Unknown history visibility; hiding the event",
            );

            false
        }
    }
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn user_can_see_state_events(&self, user_id: &UserId, room_id: &RoomId) -> bool {
    if self.services.state_cache.is_joined(user_id, room_id).await {
        return true;
    }

    let history_visibility = self
        .room_state_get_content(room_id, &StateEventType::RoomHistoryVisibility, "")
        .await
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        );

    match history_visibility {
        HistoryVisibility::Invited => self.services.state_cache.is_invited(user_id, room_id).await,
        HistoryVisibility::WorldReadable => true,
        _ => false,
    }
}

#[implement(super::Service)]
pub async fn power_level_context(
    &self,
    room_id: &RoomId,
) -> (AuthorizationRules, Vec<OwnedUserId>) {
    let Ok(create) = self
        .room_state_get(room_id, &StateEventType::RoomCreate, "")
        .await
    else {
        return (AuthorizationRules::V1, Vec::new());
    };

    let Ok(content) = create.get_content::<RoomCreateEventContent>() else {
        return (AuthorizationRules::V1, Vec::new());
    };

    let rules = content
        .room_version
        .rules()
        .map_or(AuthorizationRules::V1, |rules| rules.authorization);

    let mut creators = Vec::with_capacity(1 + content.additional_creators.len());
    creators.push(create.sender.clone());
    creators.extend(content.additional_creators.iter().cloned());

    (rules, creators)
}

#[implement(super::Service)]
pub async fn room_power_levels(&self, room_id: &RoomId) -> RoomPowerLevels {
    let content = self
        .room_state_get_content::<RoomPowerLevelsEventContent>(
            room_id,
            &StateEventType::RoomPowerLevels,
            "",
        )
        .await
        .ok();

    let (rules, creators) = self.power_level_context(room_id).await;

    RoomPowerLevels::new(RoomPowerLevelsSource::from(content), &rules, creators)
}

#[implement(super::Service)]
pub async fn get_power_levels(&self, room_id: &RoomId) -> Result<RoomPowerLevels> {
    let content = self
        .room_state_get_content::<RoomPowerLevelsEventContent>(
            room_id,
            &StateEventType::RoomPowerLevels,
            "",
        )
        .await?;

    let (rules, creators) = self.power_level_context(room_id).await;

    Ok(RoomPowerLevels::new(
        RoomPowerLevelsSource::Original(content),
        &rules,
        creators,
    ))
}
