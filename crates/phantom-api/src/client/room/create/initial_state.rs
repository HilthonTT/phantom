use futures::FutureExt;
use itertools::Itertools;
use phantom_core::{
    Result, debug_warn,
    matrix::{StateKey, pdu::PduBuilder},
};
use phantom_service::{Services, rooms::state::RoomMutexGuard};
use ruma::{
    EventEncryptionAlgorithm, RoomId, UserId,
    api::client::room::create_room::{self, v3::RoomPreset},
    events::{
        StateEventType,
        room::{
            encryption::RoomEncryptionEventContent,
            guest_access::{GuestAccess, RoomGuestAccessEventContent},
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            name::RoomNameEventContent,
            topic::RoomTopicEventContent,
        },
    },
    serde::JsonObject,
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, value::RawValue as RawJsonValue};

use crate::router::Ruma;

#[derive(Deserialize)]
pub(super) struct InitialEvent {
    #[serde(rename = "type")]
    event_type: StateEventType,

    #[serde(default = "StateKey::new")]
    state_key: StateKey,

    content: Box<RawJsonValue>,
}

impl From<InitialEvent> for PduBuilder {
    fn from(value: InitialEvent) -> Self {
        Self {
            event_type: value.event_type.into(),
            content: value.content,
            unsigned: None,
            state_key: Some(value.state_key),
            redacts: None,
            timestamp: None,
        }
    }
}

fn take_initial(
    initial_state: &mut Vec<InitialEvent>,
    event_type: &StateEventType,
    state_key: &str,
) -> Option<InitialEvent> {
    initial_state
        .extract_if(.., |event| {
            &event.event_type == event_type && event.state_key == state_key
        })
        .next()
}

pub(super) async fn apply_preset_state_pdus(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    preset: &RoomPreset,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result<Vec<InitialEvent>> {
    let mut initial_state = body
        .initial_state
        .iter()
        .map(|state| Ok(state.deserialize_as_unchecked::<InitialEvent>()?))
        .filter_ok(|event| {
            services.config.client.allow_encryption
                || event.event_type != StateEventType::RoomEncryption
        })
        .filter_ok(|event| {
            // client/appservice workaround: if a user sends an initial_state event with a
            // state event in there with the content of literally `{}` (not null or empty
            // string), let's just skip it over and warn.
            if event.content.get() == "{}" {
                debug_warn!(
                    "skipping empty initial state event of type {}",
                    event.event_type
                );
                false
            } else {
                true
            }
        })
        .filter_ok(|event| body.name.is_none() || event.event_type != StateEventType::RoomName)
        .filter_ok(|event| body.topic.is_none() || event.event_type != StateEventType::RoomTopic)
        .collect::<Result<Vec<_>>>()?;

    let join_rule_pdubuilder = take_initial(&mut initial_state, &StateEventType::RoomJoinRules, "")
        .map(Into::into)
        .unwrap_or_else(|| {
            PduBuilder::state(
                String::new(),
                &RoomJoinRulesEventContent::new(match preset {
                    RoomPreset::PublicChat => JoinRule::Public,
                    // according to spec "invite" is the default
                    _ => JoinRule::Invite,
                }),
            )
        });

    let history_visibility_pdubuilder = take_initial(
        &mut initial_state,
        &StateEventType::RoomHistoryVisibility,
        "",
    )
    .map(Into::into)
    .unwrap_or_else(|| {
        PduBuilder::state(
            String::new(),
            &RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared),
        )
    });

    let guest_access = guest_access_pdu(
        take_initial(&mut initial_state, &StateEventType::RoomGuestAccess, "").map(Into::into),
        preset,
    );

    // 5.1 Join Rules
    services
        .rooms
        .timeline
        .build_and_append_pdu(join_rule_pdubuilder, sender_user, room_id, state_lock)
        .boxed()
        .await?;

    // 5.2 History Visibility
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            history_visibility_pdubuilder,
            sender_user,
            room_id,
            state_lock,
        )
        .boxed()
        .await?;

    // 5.3 Guest Access
    if let Some(guest_access) = guest_access {
        services
            .rooms
            .timeline
            .build_and_append_pdu(guest_access, sender_user, room_id, state_lock)
            .boxed()
            .await?;
    }

    Ok(initial_state)
}

fn guest_access_pdu(initial: Option<PduBuilder>, preset: &RoomPreset) -> Option<PduBuilder> {
    let can_join = || {
        PduBuilder::state(
            String::new(),
            &RoomGuestAccessEventContent::new(GuestAccess::CanJoin),
        )
    };

    initial.or_else(|| preset.ne(&RoomPreset::PublicChat).then(can_join))
}

pub(super) async fn apply_initial_state_pdus(
    services: &Services,
    initial_state: Vec<InitialEvent>,
    preset: &RoomPreset,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let is_encrypted = encrypts_room(&initial_state);

    for event in initial_state {
        services
            .rooms
            .timeline
            .build_and_append_pdu(event.into(), sender_user, room_id, state_lock)
            .boxed()
            .await?;
    }

    if !services.config.client.allow_encryption || is_encrypted {
        return Ok(());
    }

    let config = services
        .config
        .client
        .encryption_enabled_by_default_for_room_type
        .as_deref();

    let should_encrypt = match config {
        Some("all") => true,
        Some("invite") => matches!(
            preset,
            RoomPreset::PrivateChat | RoomPreset::TrustedPrivateChat
        ),
        _ => false,
    };

    if !should_encrypt {
        return Ok(());
    }

    let algorithm = EventEncryptionAlgorithm::MegolmV1AesSha2;
    let content = RoomEncryptionEventContent::new(algorithm);
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &content),
            sender_user,
            room_id,
            state_lock,
        )
        .boxed()
        .await?;

    Ok(())
}

/// Whether `initial_state` already configures the room's encryption.
///
/// The last entry at the empty state key is the one that survives state
/// resolution, so it alone decides whether the server's forced default is
/// displaced, and only by naming a string `algorithm`. The raw field is read
/// rather than deserialized so an escaped string still counts as one.
fn encrypts_room(initial_state: &[InitialEvent]) -> bool {
    initial_state
        .iter()
        .rfind(|event| {
            event.event_type == StateEventType::RoomEncryption && event.state_key.is_empty()
        })
        .and_then(|event| serde_json::from_str::<JsonObject>(event.content.get()).ok())
        .is_some_and(|content| matches!(content.get("algorithm"), Some(JsonValue::String(_))))
}

pub(super) async fn apply_name_and_topic_pdus(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result {
    if let Some(name) = &body.name {
        services
            .rooms
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(String::new(), &RoomNameEventContent::new(name.clone())),
                sender_user,
                room_id,
                state_lock,
            )
            .boxed()
            .await?;
    }

    if let Some(topic) = &body.topic {
        services
            .rooms
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(String::new(), &RoomTopicEventContent::new(topic.clone())),
                sender_user,
                room_id,
                state_lock,
            )
            .boxed()
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruma::events::TimelineEventType;

    fn guest_access(pdu: &PduBuilder) -> GuestAccess {
        serde_json::from_str::<RoomGuestAccessEventContent>(pdu.content.get())
            .expect("guest access content")
            .guest_access
    }

    #[test]
    fn public_chat_omits_default_guest_access() {
        assert!(guest_access_pdu(None, &RoomPreset::PublicChat).is_none());
    }

    #[test]
    fn private_presets_default_to_guest_access() {
        for preset in [RoomPreset::PrivateChat, RoomPreset::TrustedPrivateChat] {
            let pdu = guest_access_pdu(None, &preset).expect("guest access pdu");

            assert_eq!(pdu.event_type, TimelineEventType::RoomGuestAccess);
            assert_eq!(pdu.state_key.as_deref(), Some(""));
            assert_eq!(guest_access(&pdu), GuestAccess::CanJoin);
        }
    }

    #[test]
    fn explicit_guest_access_survives_public_preset() {
        let explicit = PduBuilder::state(
            String::new(),
            &RoomGuestAccessEventContent::new(GuestAccess::Forbidden),
        );

        let pdu = guest_access_pdu(Some(explicit), &RoomPreset::PublicChat)
            .expect("explicit guest access pdu");

        assert_eq!(guest_access(&pdu), GuestAccess::Forbidden);
    }

    #[test]
    fn encryption_needs_a_string_algorithm() {
        for content in [
            r#"{"algorithm":"m.megolm.v1.aes-sha2"}"#,
            r#"{"algorithm":"\u006d.megolm.v1.aes-sha2"}"#,
            r#"{"algorithm":"whatever","rotation_period_ms":604800000}"#,
            r#"{"algorithm" : "m.megolm.v1.aes-sha2"}"#,
        ] {
            assert!(encrypts_room(&[initial_state_event(
                "m.room.encryption",
                "",
                content
            )]));
        }
    }

    fn initial_state_event(event_type: &str, state_key: &str, content: &str) -> InitialEvent {
        let json =
            format!(r#"{{"type":"{event_type}","state_key":"{state_key}","content":{content}}}"#);

        serde_json::from_str(&json).expect("initial state event")
    }

    #[test]
    fn contentless_encryption_leaves_the_forced_default() {
        for content in [
            "{}",
            "{ }",
            r#"{"x":1}"#,
            r#"{"algorithm":1}"#,
            r#"{"algorithm":null}"#,
        ] {
            assert!(!encrypts_room(&[initial_state_event(
                "m.room.encryption",
                "",
                content
            )]));
        }
    }

    #[test]
    fn the_last_entry_at_the_empty_state_key_decides() {
        let valid = r#"{"algorithm":"m.megolm.v1.aes-sha2"}"#;
        let junk = r#"{"x":1}"#;
        let event = |content| initial_state_event("m.room.encryption", "", content);

        assert!(!encrypts_room(&[event(valid), event(junk)]));
        assert!(encrypts_room(&[event(junk), event(valid)]));
        assert!(encrypts_room(&[
            event(valid),
            initial_state_event("m.room.encryption", "x", junk)
        ]));
    }

    #[test]
    fn a_foreign_state_key_never_encrypts() {
        let content = r#"{"algorithm":"m.megolm.v1.aes-sha2"}"#;
        let event = initial_state_event("m.room.encryption", "x", content);

        assert!(!encrypts_room(&[event]));
    }

    #[test]
    fn other_event_types_never_encrypt() {
        let event =
            initial_state_event("m.room.name", "", r#"{"algorithm":"m.megolm.v1.aes-sha2"}"#);

        assert!(!encrypts_room(&[event]));
    }
}
