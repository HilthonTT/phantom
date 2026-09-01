mod levels;
mod membership;

use std::{borrow::Borrow, collections::BTreeSet};

use futures::{
    Future,
    future::{OptionFuture, join3},
};
use ruma::{
    Int, OwnedUserId, RoomVersionId, UserId,
    events::room::{
        create::RoomCreateEventContent,
        join_rules::{JoinRule, RoomJoinRulesEventContent},
        member::{MembershipState, ThirdPartyInvite},
        power_levels::RoomPowerLevelsEventContent,
        third_party_invite::RoomThirdPartyInviteEventContent,
    },
    int,
    serde::{Base64, Raw, base64::Standard},
    third_party_invite::IdentityServerBase64PublicKey,
};
use serde::{
    Deserialize,
    de::{Error as _, IgnoredAny},
};
use serde_json::{from_str as from_json_str, value::RawValue as RawJsonValue};

use self::{
    levels::{can_send_event, check_power_levels, check_redaction},
    membership::valid_membership_change,
};
use super::{
    Error, Event, Result, StateEventType, StateKey, TimelineEventType,
    power_levels::{
        deserialize_power_levels, deserialize_power_levels_content_fields,
        deserialize_power_levels_content_invite, deserialize_power_levels_content_redact,
    },
    room_version::RoomVersion,
};
use crate::{debug, error, trace, warn};

/// `m.room.aliases` was dropped from ruma's `TimelineEventType`, but room
/// versions 6 and below still give it special auth rules, so the check compares
/// against the wire type directly.
const ROOM_ALIASES_TYPE: &str = "m.room.aliases";

#[derive(Deserialize)]
struct GetMembership {
    membership: MembershipState,
}

#[derive(Deserialize)]
struct RoomMemberContentFields {
    membership: Option<Raw<MembershipState>>,
    join_authorised_via_users_server: Option<Raw<OwnedUserId>>,
}

/// For the given event `kind` what are the relevant auth events that are needed
/// to authenticate this `content`.
///
/// # Errors
///
/// This function will return an error if the supplied `content` is not a JSON
/// object.
pub fn auth_types_for_event(
    kind: &TimelineEventType,
    sender: &UserId,
    state_key: Option<&str>,
    content: &RawJsonValue,
) -> serde_json::Result<Vec<(StateEventType, StateKey)>> {
    if kind == &TimelineEventType::RoomCreate {
        return Ok(vec![]);
    }

    let mut auth_types = vec![
        (StateEventType::RoomPowerLevels, StateKey::new()),
        (StateEventType::RoomMember, sender.as_str().into()),
        (StateEventType::RoomCreate, StateKey::new()),
    ];

    if kind == &TimelineEventType::RoomMember {
        #[derive(Deserialize)]
        struct RoomMemberContentFields {
            membership: Option<Raw<MembershipState>>,
            third_party_invite: Option<Raw<ThirdPartyInvite>>,
            join_authorised_via_users_server: Option<Raw<OwnedUserId>>,
        }

        if let Some(state_key) = state_key {
            let content: RoomMemberContentFields = from_json_str(content.get())?;

            if let Some(Ok(membership)) = content.membership.map(|m| m.deserialize()) {
                if [
                    MembershipState::Join,
                    MembershipState::Invite,
                    MembershipState::Knock,
                ]
                .contains(&membership)
                {
                    let key = (StateEventType::RoomJoinRules, StateKey::new());
                    if !auth_types.contains(&key) {
                        auth_types.push(key);
                    }

                    if let Some(Ok(u)) = content
                        .join_authorised_via_users_server
                        .map(|m| m.deserialize())
                    {
                        let key = (StateEventType::RoomMember, u.as_str().into());
                        if !auth_types.contains(&key) {
                            auth_types.push(key);
                        }
                    }
                }

                let key = (StateEventType::RoomMember, state_key.into());
                if !auth_types.contains(&key) {
                    auth_types.push(key);
                }

                if membership == MembershipState::Invite
                    && let Some(Ok(t_id)) = content.third_party_invite.map(|t| t.deserialize())
                    && let Ok(signed) = t_id.signed.deserialize()
                {
                    let key = (StateEventType::RoomThirdPartyInvite, signed.token.into());
                    if !auth_types.contains(&key) {
                        auth_types.push(key);
                    }
                }
            }
        }
    }

    Ok(auth_types)
}

/// Authenticate the incoming `event`.
///
/// The steps of authentication are:
///
/// * check that the event is being authenticated for the correct room
/// * then there are checks for specific event types
///
/// The `fetch_state` closure should gather state from a state snapshot. We need
/// to know if the event passes auth against some state not a recursive
/// collection of auth_events fields.
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		event_id = incoming_event.event_id().borrow().as_str()
	)
)]
pub async fn auth_check<F, Fut, Fetched, Incoming>(
    room_version: &RoomVersion,
    incoming_event: &Incoming,
    current_third_party_invite: Option<&Incoming>,
    fetch_state: F,
) -> Result<bool, Error>
where
    F: Fn(&StateEventType, &str) -> Fut + Send,
    Fut: Future<Output = Option<Fetched>> + Send,
    Fetched: Event + Send,
    Incoming: Event + Send + Sync,
{
    debug!(
        "auth_check beginning for {} ({})",
        incoming_event.event_id(),
        incoming_event.event_type()
    );

    let sender = incoming_event.sender();

    if *incoming_event.event_type() == TimelineEventType::RoomCreate {
        #[derive(Deserialize)]
        struct RoomCreateContentFields {
            room_version: Option<Raw<RoomVersionId>>,
            creator: Option<Raw<IgnoredAny>>,
        }

        debug!("start m.room.create check");

        if incoming_event.prev_events().next().is_some() {
            warn!("the room creation event had previous events");
            return Ok(false);
        }

        let Some(room_id_server_name) = incoming_event.room_id().server_name() else {
            warn!("room ID has no servername");
            return Ok(false);
        };

        if room_id_server_name != sender.server_name() {
            warn!("servername of room ID does not match servername of sender");
            return Ok(false);
        }

        let content: RoomCreateContentFields = from_json_str(incoming_event.content().get())?;
        if content
            .room_version
            .is_some_and(|v| v.deserialize().is_err())
        {
            warn!("invalid room version found in m.room.create event");
            return Ok(false);
        }

        if !room_version.use_room_create_sender && content.creator.is_none() {
            warn!("no creator field found in m.room.create content");
            return Ok(false);
        }

        debug!("m.room.create event was allowed");
        return Ok(true);
    }

    let (room_create_event, power_levels_event, sender_member_event) = join3(
        fetch_state(&StateEventType::RoomCreate, ""),
        fetch_state(&StateEventType::RoomPowerLevels, ""),
        fetch_state(&StateEventType::RoomMember, sender.as_str()),
    )
    .await;

    let room_create_event = match room_create_event {
        None => {
            warn!("no m.room.create event in auth chain");
            return Ok(false);
        }
        Some(e) => e,
    };

    if !incoming_event
        .auth_events()
        .any(|id| id.borrow() == room_create_event.event_id().borrow())
    {
        warn!("no m.room.create event in auth events");
        return Ok(false);
    }

    #[derive(Deserialize)]
    #[allow(clippy::items_after_statements)]
    struct RoomCreateContentFederate {
        #[serde(rename = "m.federate", default = "ruma::serde::default_true")]
        federate: bool,
    }
    let room_create_content: RoomCreateContentFederate =
        from_json_str(room_create_event.content().get())?;
    if !room_create_content.federate
        && room_create_event.sender().server_name() != incoming_event.sender().server_name()
    {
        warn!(
            "room is not federated and event's sender domain does not match create event's \
			 sender domain"
        );
        return Ok(false);
    }

    if room_version.special_case_aliases_auth
        && incoming_event.event_type().to_string() == ROOM_ALIASES_TYPE
    {
        debug!("starting m.room.aliases check");

        if incoming_event.state_key() != Some(sender.server_name().as_str()) {
            warn!("state_key does not match sender");
            return Ok(false);
        }

        debug!("m.room.aliases event was allowed");
        return Ok(true);
    }

    if *incoming_event.event_type() == TimelineEventType::RoomMember {
        debug!("starting m.room.member check");
        let state_key = match incoming_event.state_key() {
            None => {
                warn!("no statekey in member event");
                return Ok(false);
            }
            Some(s) => s,
        };

        let content: RoomMemberContentFields = from_json_str(incoming_event.content().get())?;
        if content
            .membership
            .as_ref()
            .and_then(|m| m.deserialize().ok())
            .is_none()
        {
            warn!("no valid membership field found for m.room.member event content");
            return Ok(false);
        }

        let target_user =
            <&UserId>::try_from(state_key).map_err(|e| Error::InvalidPdu(format!("{e}")))?;

        let user_for_join_auth = content
            .join_authorised_via_users_server
            .as_ref()
            .and_then(|u| u.deserialize().ok());

        let user_for_join_auth_event: OptionFuture<_> = user_for_join_auth
            .as_ref()
            .map(|auth_user| fetch_state(&StateEventType::RoomMember, auth_user.as_str()))
            .into();

        let target_user_member_event =
            fetch_state(&StateEventType::RoomMember, target_user.as_str());

        let join_rules_event = fetch_state(&StateEventType::RoomJoinRules, "");

        let (join_rules_event, target_user_member_event, user_for_join_auth_event) = join3(
            join_rules_event,
            target_user_member_event,
            user_for_join_auth_event,
        )
        .await;

        let user_for_join_auth_membership = user_for_join_auth_event
            .and_then(|mem| from_json_str::<GetMembership>(mem?.content().get()).ok())
            .map_or(MembershipState::Leave, |mem| mem.membership);

        if !valid_membership_change(
            room_version,
            target_user,
            target_user_member_event.as_ref(),
            sender,
            sender_member_event.as_ref(),
            incoming_event,
            current_third_party_invite,
            power_levels_event.as_ref(),
            join_rules_event.as_ref(),
            user_for_join_auth.as_deref(),
            &user_for_join_auth_membership,
            &room_create_event,
        )? {
            return Ok(false);
        }

        debug!("m.room.member event was allowed");
        return Ok(true);
    }

    #[allow(clippy::manual_let_else)]
    let sender_member_event = match sender_member_event {
        Some(mem) => mem,
        None => {
            warn!("sender not found in room");
            return Ok(false);
        }
    };

    let sender_membership_event_content: RoomMemberContentFields =
        from_json_str(sender_member_event.content().get())?;
    let membership_state = sender_membership_event_content
        .membership
        .expect("we should test before that this field exists")
        .deserialize()?;

    if !matches!(membership_state, MembershipState::Join) {
        warn!("sender's membership is not join");
        return Ok(false);
    }

    let sender_power_level = match &power_levels_event {
        Some(pl) => {
            let content =
                deserialize_power_levels_content_fields(pl.content().get(), room_version)?;
            match content.get_user_power(sender) {
                Some(level) => *level,
                _ => content.users_default,
            }
        }
        _ => {
            let is_creator = if room_version.use_room_create_sender {
                room_create_event.sender() == sender
            } else {
                #[allow(deprecated)]
                from_json_str::<RoomCreateEventContent>(room_create_event.content().get())
                    .is_ok_and(|create| create.creator.unwrap() == *sender)
            };

            if is_creator { int!(100) } else { int!(0) }
        }
    };

    if *incoming_event.event_type() == TimelineEventType::RoomThirdPartyInvite {
        let invite_level = match &power_levels_event {
            Some(power_levels) => {
                deserialize_power_levels_content_invite(power_levels.content().get(), room_version)?
                    .invite
            }
            None => int!(0),
        };

        if sender_power_level < invite_level {
            warn!("sender's cannot send invites in this room");
            return Ok(false);
        }

        debug!("m.room.third_party_invite event was allowed");
        return Ok(true);
    }

    if !can_send_event(
        incoming_event,
        power_levels_event.as_ref(),
        sender_power_level,
    ) {
        warn!("user cannot send event");
        return Ok(false);
    }

    if *incoming_event.event_type() == TimelineEventType::RoomPowerLevels {
        debug!("starting m.room.power_levels check");

        match check_power_levels(
            room_version,
            incoming_event,
            power_levels_event.as_ref(),
            sender_power_level,
        ) {
            Some(required_pwr_lvl) => {
                if !required_pwr_lvl {
                    warn!("m.room.power_levels was not allowed");
                    return Ok(false);
                }
            }
            _ => {
                warn!("m.room.power_levels was not allowed");
                return Ok(false);
            }
        }
        debug!("m.room.power_levels event allowed");
    }

    if room_version.extra_redaction_checks
        && *incoming_event.event_type() == TimelineEventType::RoomRedaction
    {
        let redact_level = match power_levels_event {
            Some(pl) => {
                deserialize_power_levels_content_redact(pl.content().get(), room_version)?.redact
            }
            None => int!(50),
        };

        if !check_redaction(
            room_version,
            incoming_event,
            sender_power_level,
            redact_level,
        )? {
            return Ok(false);
        }
    }

    debug!("allowing event passed all checks");
    Ok(true)
}

#[cfg(test)]
mod tests;
