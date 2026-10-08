mod validate;

use axum::extract::State;
use futures::{FutureExt, TryStreamExt};
use phantom_core::{
    Err, Result, err,
    matrix::pdu::{PduBuilder, PduEvent},
    stream::TryBroadbandExt,
};
use phantom_service::Services;
use ruma::{
    CanonicalJsonObject, MilliSecondsSinceUnixEpoch, OwnedEventId, RoomId, UserId,
    api::client::state::{
        get_state_event_for_key::{self, v3::StateEventFormat},
        get_state_events, send_state_event,
    },
    events::{AnyStateEventContent, StateEventType},
    serde::Raw,
};
use serde_json::{json, value::to_raw_value};

use self::validate::allowed_to_send_state_event;
use crate::{
    client::with_membership,
    router::{Ruma, RumaResponse},
};

/// # `PUT /_matrix/client/*/rooms/{roomId}/state/{eventType}/{stateKey}`
///
/// Sends a state event into the room.
pub(crate) async fn send_state_event_for_key_route(
    State(services): State<crate::router::State>,
    body: Ruma<send_state_event::v3::Request>,
) -> Result<send_state_event::v3::Response> {
    let sender_user = body.sender_user();

    let event_id = send_state_event_for_key_helper(
        &services,
        sender_user,
        &body.room_id,
        &body.event_type,
        &body.body.body,
        &body.state_key,
        if body.appservice_info.is_some() {
            body.timestamp
        } else {
            None
        },
    )
    .await?;

    Ok(send_state_event::v3::Response::new(event_id))
}

/// # `PUT /_matrix/client/*/rooms/{roomId}/state/{eventType}`
///
/// Sends a state event into the room.
pub(crate) async fn send_state_event_for_empty_key_route(
    State(services): State<crate::router::State>,
    body: Ruma<send_state_event::v3::Request>,
) -> Result<RumaResponse<send_state_event::v3::Response>> {
    send_state_event_for_key_route(State(services), body)
        .boxed()
        .await
        .map(RumaResponse)
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state`
///
/// Get all state events for a room.
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_state_events::v3::Request>,
) -> Result<get_state_events::v3::Response> {
    let sender_user = body.sender_user();

    if !services
        .rooms
        .state_accessor
        .user_can_see_state_events(sender_user, &body.room_id)
        .await
    {
        return Err!(Request(Forbidden(
            "You don't have permission to view the room state."
        )));
    }

    let encrypted = services
        .rooms
        .state_accessor
        .is_encrypted_room(&body.room_id)
        .await;

    let room_state = services
        .rooms
        .state_accessor
        .room_state_full_pdus(&body.room_id)
        .broad_and_then(async |pdu| {
            Ok(with_membership(&services, pdu, sender_user, encrypted).await)
        })
        .map_ok(PduEvent::into_state_event)
        .try_collect()
        .await?;

    Ok(get_state_events::v3::Response::new(room_state))
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state/{eventType}/{stateKey}`
///
/// Get single state event of a room with the specified state key.
/// The optional query parameter `?format=event|content` allows returning the
/// full room state event or just the state event's content (default behaviour)
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_for_key_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_state_event_for_key::v3::Request>,
) -> Result<get_state_event_for_key::v3::Response> {
    let sender_user = body.sender_user();

    if !services
        .rooms
        .state_accessor
        .user_can_see_state_events(sender_user, &body.room_id)
        .await
    {
        return Err!(Request(NotFound(debug_warn!(
            "You don't have permission to view the room state."
        ))));
    }

    let event = services
        .rooms
        .state_accessor
        .room_state_get(&body.room_id, &body.event_type, &body.state_key)
        .await
        .map_err(|e| {
            err!(Request(NotFound(debug_warn!(
                message = format_args!("Failed to get state event: {e}."),
                room_id = ?body.room_id,
                event_type = ?body.event_type
            ))))
        })?;

    let event_or_content = match body.format {
        StateEventFormat::Event => json!({
            "content": event.content,
            "event_id": event.event_id,
            "origin_server_ts": event.origin_server_ts,
            "room_id": event.room_id,
            "sender": event.sender,
            "state_key": event.state_key,
            "type": event.kind,
            "unsigned": event.unsigned,
        }),

        _ => event.get_content_as_value(),
    };

    let event_or_content = to_raw_value(&event_or_content).expect("serializable JSON value");

    Ok(get_state_event_for_key::v3::Response::new(event_or_content))
}

/// # `GET /_matrix/client/v3/rooms/{roomid}/state/{eventType}`
///
/// Get single state event of a room.
/// The optional query parameter `?format=event|content` allows returning the
/// full room state event or just the state event's content (default behaviour)
///
/// - If not joined: Only works if current room history visibility is world
///   readable
pub(crate) async fn get_state_events_for_empty_key_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_state_event_for_key::v3::Request>,
) -> Result<RumaResponse<get_state_event_for_key::v3::Response>> {
    get_state_events_for_key_route(State(services), body)
        .await
        .map(RumaResponse)
}

async fn send_state_event_for_key_helper(
    services: &Services,
    sender: &UserId,
    room_id: &RoomId,
    event_type: &StateEventType,
    json: &Raw<AnyStateEventContent>,
    state_key: &str,
    timestamp: Option<MilliSecondsSinceUnixEpoch>,
) -> Result<OwnedEventId> {
    allowed_to_send_state_event(services, room_id, event_type, state_key, json).await?;
    let state_lock = services.rooms.state.mutex.lock(room_id).await;

    let current = match state_dedup_eligible(event_type, timestamp.as_ref()) {
        false => None,
        true => match services
            .rooms
            .state_accessor
            .room_state_get(room_id, event_type, state_key)
            .await
        {
            Ok(current) => Some(current),
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e),
        },
    };

    if let Some(current) = current
        && current.sender == sender
    {
        let content = json.deserialize_as_unchecked::<CanonicalJsonObject>()?;

        if is_duplicate_state(event_type, sender, &content, &current)?
            && services.rooms.state_cache.is_joined(sender, room_id).await
        {
            return Ok(current.event_id.clone());
        }
    }

    let event_id = services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                event_type: event_type.to_string().into(),
                content: serde_json::from_str(json.json().get())?,
                state_key: Some(state_key.into()),
                timestamp,
                ..Default::default()
            },
            sender,
            room_id,
            &state_lock,
        )
        .boxed()
        .await?;

    Ok(event_id)
}

fn state_dedup_eligible(
    event_type: &StateEventType,
    timestamp: Option<&MilliSecondsSinceUnixEpoch>,
) -> bool {
    timestamp.is_none() && !matches!(event_type, StateEventType::RoomMember)
}

/// Whether an incoming state event is a content-identical resend by its own
/// author.
///
/// The caller's guard returns before `state_res::auth_check` runs, so every
/// conjunct gating that early return must be a version-invariant fact that can
/// only suppress a dedup, never permit one. Membership class qualifies; power
/// levels and per-type rules do not, and wanting an exact status code there is
/// a reason to move the guard after authorization rather than to add a
/// conjunct.
fn is_duplicate_state(
    event_type: &StateEventType,
    sender: &UserId,
    content: &CanonicalJsonObject,
    current: &PduEvent,
) -> Result<bool> {
    if matches!(event_type, StateEventType::RoomMember) || current.sender != sender {
        return Ok(false);
    }

    let current_content: CanonicalJsonObject = serde_json::from_str(current.content.get())?;

    Ok(current_content == *content)
}

#[cfg(test)]
mod tests {
    use ruma::user_id;
    use serde_json::{Value as JsonValue, from_str, from_value};

    use super::*;

    fn current_state(sender: &str, content: &JsonValue) -> PduEvent {
        from_value(json!({
            "type": "m.room.history_visibility",
            "content": content,
            "state_key": "",
            "event_id": "$event:example.com",
            "room_id": "!room:example.com",
            "sender": sender,
            "prev_events": [],
            "auth_events": [],
            "origin_server_ts": 1,
            "depth": 1,
            "hashes": { "sha256": "thishashcoversallfieldsincasethisisredacted" },
        }))
        .expect("valid pdu")
    }

    #[test]
    fn identical_state_content_is_duplicate() {
        let sender = user_id!("@alice:example.com");
        let current = current_state(
            sender.as_str(),
            &json!({ "history_visibility": "shared", "extra": true }),
        );

        let content =
            from_str::<CanonicalJsonObject>(r#"{ "extra": true, "history_visibility": "shared" }"#)
                .expect("canonical content");

        assert!(
            is_duplicate_state(
                &StateEventType::RoomHistoryVisibility,
                sender,
                &content,
                &current,
            )
            .expect("comparison")
        );
    }

    #[test]
    fn changed_state_content_is_not_duplicate() {
        let sender = user_id!("@alice:example.com");
        let current = current_state(sender.as_str(), &json!({ "history_visibility": "shared" }));
        let content =
            from_str(r#"{ "history_visibility": "world_readable" }"#).expect("canonical content");

        assert!(
            !is_duplicate_state(
                &StateEventType::RoomHistoryVisibility,
                sender,
                &content,
                &current,
            )
            .expect("comparison")
        );
    }

    #[test]
    fn different_sender_is_not_duplicate() {
        let current = current_state(
            "@alice:example.com",
            &json!({ "history_visibility": "shared" }),
        );

        let content = from_str(r#"{ "history_visibility": "shared" }"#).expect("canonical content");

        assert!(
            !is_duplicate_state(
                &StateEventType::RoomHistoryVisibility,
                user_id!("@bob:example.com"),
                &content,
                &current,
            )
            .expect("comparison")
        );
    }

    #[test]
    fn member_state_is_not_duplicate() {
        let sender = user_id!("@alice:example.com");
        let current = current_state(sender.as_str(), &json!({ "membership": "join" }));
        let content = from_str(r#"{ "membership": "join" }"#).expect("canonical content");

        assert!(
            !is_duplicate_state(&StateEventType::RoomMember, sender, &content, &current)
                .expect("comparison")
        );
    }

    #[test]
    fn timestamped_state_is_not_eligible_for_dedup() {
        let event_type = StateEventType::RoomHistoryVisibility;
        let timestamp = MilliSecondsSinceUnixEpoch::now();

        assert!(state_dedup_eligible(&event_type, None));
        assert!(!state_dedup_eligible(&event_type, Some(&timestamp)));
    }
}
