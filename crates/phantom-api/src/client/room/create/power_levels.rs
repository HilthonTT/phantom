use std::collections::BTreeMap;

use phantom_core::{
    Result,
    bool::BoolExt,
    err,
    matrix::{StateKey, pdu::PduBuilder},
};
use phantom_service::{Services, rooms::state::RoomMutexGuard};
use ruma::{
    Int, OwnedUserId, RoomId, UserId,
    api::client::room::create_room::{self, RoomPowerLevelsContentOverride, v3::RoomPreset},
    events::{TimelineEventType, room::power_levels::RoomPowerLevelsEventContent},
    int,
    room_version_rules::RoomVersionRules,
    serde::{JsonObject, Raw},
};
use serde_json::{Value as JsonValue, json, value::to_raw_value};

use super::invites::{ignored_users, invite_allowed};
use crate::router::Ruma;

pub(super) async fn apply_power_levels_pdu(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    preset: &RoomPreset,
    version_rules: &RoomVersionRules,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let users = build_power_levels_users(services, body, preset, version_rules, sender_user).await;

    let default_override = services
        .config
        .client
        .default_power_level_content_override
        .as_ref();

    let power_levels_content = default_power_levels_content(
        version_rules,
        default_override,
        body.power_level_content_override.as_ref(),
        preset,
        users,
    )?;

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomPowerLevels,
                content: to_raw_value(&power_levels_content)?,
                state_key: Some(StateKey::new()),
                ..Default::default()
            },
            sender_user,
            room_id,
            state_lock,
        )
        .await
        .map(|_| ())
}

async fn build_power_levels_users(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    preset: &RoomPreset,
    version_rules: &RoomVersionRules,
    sender_user: &UserId,
) -> BTreeMap<OwnedUserId, Int> {
    let seed = version_rules
        .authorization
        .explicitly_privilege_room_creators
        .or(|| (sender_user.to_owned(), int!(100)))
        .into_iter()
        .collect::<BTreeMap<_, _>>();

    let trusted_invitees = *preset == RoomPreset::TrustedPrivateChat
        && !version_rules.authorization.additional_room_creators;

    if !trusted_invitees {
        return seed;
    }

    let ignored = ignored_users(services, sender_user).await;

    body.invite
        .iter()
        .filter(|invite| invite_allowed(ignored.as_ref(), invite))
        .fold(seed, |mut users, invite| {
            users.insert(invite.clone(), int!(100));
            users
        })
}

/// creates the power_levels_content for the PDU builder
fn default_power_levels_content(
    version_rules: &RoomVersionRules,
    default_power_level_content_override: Option<&JsonValue>,
    power_level_content_override: Option<&Raw<RoomPowerLevelsContentOverride>>,
    preset: &RoomPreset,
    users: BTreeMap<OwnedUserId, Int>,
) -> Result<JsonValue> {
    use serde_json::to_value;

    let mut power_levels_content = RoomPowerLevelsEventContent::new(&version_rules.authorization);
    power_levels_content.users = users;

    let mut power_levels_content = to_value(power_levels_content)?;

    // secure proper defaults of sensitive/dangerous permissions that moderators
    // (power level 50) should not have easy access to
    power_levels_content["events"]["m.room.power_levels"] = json!(100);
    power_levels_content["events"]["m.room.server_acl"] = json!(100);
    power_levels_content["events"]["m.room.encryption"] = json!(100);
    power_levels_content["events"]["m.room.history_visibility"] = json!(100);

    if version_rules
        .authorization
        .explicitly_privilege_room_creators
    {
        power_levels_content["events"]["m.room.tombstone"] = json!(150);
    } else {
        power_levels_content["events"]["m.room.tombstone"] = json!(100);
    }

    // always allow users to respond (not post new) to polls. this is primarily
    // useful in read-only announcement rooms that post a public poll.
    power_levels_content["events"]["org.matrix.msc3381.poll.response"] = json!(0);
    power_levels_content["events"]["m.poll.response"] = json!(0);

    // public_chat: pin invite and call-setup events at PL 50. Synapse pins
    // invite and m.call.invite here; the MSC3401 entries are tuwunel-only.
    if *preset == RoomPreset::PublicChat {
        power_levels_content["invite"] = json!(50);
        power_levels_content["events"]["m.call.invite"] = json!(50);
        power_levels_content["events"]["m.call"] = json!(50);
        power_levels_content["events"]["m.call.member"] = json!(50);
        power_levels_content["events"]["org.matrix.msc3401.call"] = json!(50);
        power_levels_content["events"]["org.matrix.msc3401.call.member"] = json!(50);
    }

    if let Some(default_power_level_content_override) = default_power_level_content_override {
        let overrides = default_power_level_content_override
            .as_object()
            .expect("default_power_level_content_override is validated at startup")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()));

        merge_power_level_content_override(&mut power_levels_content, overrides);
    }

    if let Some(power_level_content_override) = power_level_content_override {
        let overrides: JsonObject = serde_json::from_str(power_level_content_override.json().get())
            .map_err(|e| {
                err!(Request(BadJson(
                    "Invalid power_level_content_override: {e:?}"
                )))
            })?;

        merge_power_level_content_override(&mut power_levels_content, overrides);
    }

    Ok(power_levels_content)
}

/// Replace each top-level power-levels key wholesale; no deep merge.
fn merge_power_level_content_override(
    power_levels_content: &mut JsonValue,
    overrides: impl IntoIterator<Item = (String, JsonValue)>,
) {
    power_levels_content
        .as_object_mut()
        .expect("power levels content must serialize to an object")
        .extend(overrides);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruma::RoomVersionId;

    #[test]
    fn default_power_levels_content_applies_server_default_override() {
        let version_rules = RoomVersionId::V11.rules().expect("supported room version");

        let content = default_power_levels_content(
            &version_rules,
            Some(&json!({ "users_default": 50 })),
            None,
            &RoomPreset::PrivateChat,
            BTreeMap::new(),
        )
        .expect("power levels content");

        assert_eq!(content["users_default"], json!(50));
    }

    #[test]
    fn request_override_wins_over_server_default_override() {
        let version_rules = RoomVersionId::V11.rules().expect("supported room version");
        let request_override =
            Raw::from_json(to_raw_value(&json!({ "users_default": 75 })).expect("raw json"));

        let content = default_power_levels_content(
            &version_rules,
            Some(&json!({ "users_default": 50 })),
            Some(&request_override),
            &RoomPreset::PrivateChat,
            BTreeMap::new(),
        )
        .expect("power levels content");

        assert_eq!(content["users_default"], json!(75));
    }

    #[test]
    fn default_override_preserves_explicit_user_power_levels() {
        let version_rules = RoomVersionId::V11.rules().expect("supported room version");
        let creator = OwnedUserId::try_from("@alice:example.com").expect("valid user id");
        let users = BTreeMap::from([(creator.clone(), int!(100))]);

        let content = default_power_levels_content(
            &version_rules,
            Some(&json!({ "users_default": 50 })),
            None,
            &RoomPreset::PrivateChat,
            users,
        )
        .expect("power levels content");

        assert_eq!(content["users_default"], json!(50));
        assert_eq!(content["users"][creator.as_str()], json!(100));
    }
}
