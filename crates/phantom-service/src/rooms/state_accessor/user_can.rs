//! What a user is allowed to do or see in a room.
//!
//! Visibility is decided against the state as it was at the event in question,
//! not as it is now: a user who has since left may still see what they could
//! see while they were there, and a room that is world-readable today does not
//! retroactively open what it hid yesterday.

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

/// Whether `sender` may redact `redacts`.
///
/// `federation` widens "their own event" to "an event from their own server",
/// which is what a redaction arriving over federation is judged by: the
/// sending server vouches for its own users.
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
        // A room with no power levels is one where the creator holds all of
        // them, so fall back on who created it.
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

/// Whether `user_id` may see `event_id`, by the room's `history_visibility` at
/// that event.
#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn user_can_see_event(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    event_id: &EventId,
) -> bool {
    // An event whose state this server never recorded cannot be judged, and
    // is left visible rather than hidden: it is reachable only to a caller
    // that already had the event id.
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
        // Non-exhaustive: a room may name a visibility this server has no
        // rule for, and the safe reading of one is to show nothing.
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

/// Whether `user_id` may read the room's current state.
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

/// The room-version rules and creator set that power levels are read against.
///
/// From room version 11 the creator is the sender of `m.room.create` rather
/// than a field in its content, and from version 12 creators hold power
/// levels the event itself never states — so both have to come from the
/// create event before a power level means anything.
///
/// A room whose create event cannot be read falls back to the v1 rules with
/// no creators, which grants nothing: an unreadable room is not one where
/// everyone is privileged.
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

    // The sender rather than the deprecated `creator` field: it is the
    // creator in every room version, and is the only spelling from v11 on.
    let mut creators = Vec::with_capacity(1 + content.additional_creators.len());
    creators.push(create.sender.clone());
    creators.extend(content.additional_creators.iter().cloned());

    (rules, creators)
}
