use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
};

use phantom_core::{
    Result, implement,
    matrix::{PduEvent, pdu::gen_event_id},
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, RoomId, RoomVersionId, UserId,
    api::federation::membership::RawStrippedState,
    events::{AnyStrippedStateEvent, StateEventType},
    room_version_rules::RoomIdFormatVersion,
    serde::{JsonObject, Raw},
};
use serde::Deserialize;

use super::Service;

type StateCell = (StateEventType, String);

type Accumulator = (HashMap<StateCell, usize>, Vec<RawStrippedState>);

#[derive(Deserialize)]
struct Cell<'a> {
    #[serde(rename = "type", borrow)]
    kind: Cow<'a, str>,

    #[serde(borrow)]
    state_key: Cow<'a, str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrippedCreateVerdict {
    Valid,
    Missing,
    NotPdu,
    WrongRoom,
    BadSignature,
}

#[must_use]
pub fn enforce_stripped_create(
    verdict: StrippedCreateVerdict,
    v12_room_ids: bool,
    enforce: bool,
) -> bool {
    match verdict {
        StrippedCreateVerdict::Valid => false,
        StrippedCreateVerdict::WrongRoom => v12_room_ids || enforce,
        StrippedCreateVerdict::Missing
        | StrippedCreateVerdict::NotPdu
        | StrippedCreateVerdict::BadSignature => enforce,
    }
}

#[must_use]
pub fn v12_room_ids(room_version: &RoomVersionId) -> bool {
    room_version
        .rules()
        .is_some_and(|rules| matches!(rules.room_id_format, RoomIdFormatVersion::V2))
}

#[must_use]
pub fn dedup_stripped_state(state: Vec<RawStrippedState>) -> Vec<RawStrippedState> {
    let (_, kept) = state
        .into_iter()
        .filter_map(|entry| state_cell(&entry).map(|cell| (cell, entry)))
        .fold(
            Accumulator::default(),
            |(mut chosen, mut kept), (cell, entry)| {
                match chosen.entry(cell) {
                    Entry::Vacant(vacant) => {
                        vacant.insert(kept.len());
                        kept.push(entry);
                    }
                    Entry::Occupied(occupied) => {
                        let held = &mut kept[*occupied.get()];

                        if is_legacy(held) && !is_legacy(&entry) {
                            *held = entry;
                        }
                    }
                }

                (chosen, kept)
            },
        );

    kept
}

pub fn without_member(
    state: Vec<RawStrippedState>,
    user_id: &UserId,
) -> impl Iterator<Item = RawStrippedState> {
    state
        .into_iter()
        .filter(move |entry| !occupies_member_cell(entry, user_id))
}

fn state_cell(state: &RawStrippedState) -> Option<StateCell> {
    cell(state).map(|cell| (cell.kind.as_ref().into(), cell.state_key.into_owned()))
}

fn occupies_member_cell(state: &RawStrippedState, user_id: &UserId) -> bool {
    cell(state).is_some_and(|cell| {
        StateEventType::from(cell.kind.as_ref()) == StateEventType::RoomMember
            && cell.state_key == user_id.as_str()
    })
}

fn cell(state: &RawStrippedState) -> Option<Cell<'_>> {
    serde_json::from_str(entry_json(state)?).ok()
}

#[expect(deprecated)]
fn entry_json(state: &RawStrippedState) -> Option<&str> {
    match state {
        RawStrippedState::Stripped(raw) => Some(raw.json().get()),
        RawStrippedState::Pdu(raw) => Some(raw.get()),
        _ => None,
    }
}

#[expect(deprecated)]
fn is_legacy(state: &RawStrippedState) -> bool {
    matches!(state, RawStrippedState::Stripped(_))
}

#[expect(deprecated)]
#[must_use]
pub fn into_client_stripped(
    room_id: &RoomId,
    state: RawStrippedState,
) -> Option<Raw<AnyStrippedStateEvent>> {
    match state {
        RawStrippedState::Stripped(raw) => Some(raw),
        RawStrippedState::Pdu(raw) => {
            let mut event: JsonObject = serde_json::from_str(raw.get()).ok()?;

            event.insert("event_id".into(), "$placeholder".into());
            event
                .entry("room_id")
                .or_insert_with(|| room_id.as_str().into());

            let pdu: PduEvent = serde_json::from_value(event.into()).ok()?;

            Some(pdu.into_stripped_state_event())
        }
        _ => None,
    }
}

#[implement(Service)]
#[expect(deprecated)]
#[tracing::instrument(level = "debug", skip_all, fields(%room_id))]
pub async fn validate_stripped_create(
    &self,
    state: &[RawStrippedState],
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Result<StrippedCreateVerdict> {
    let create = state.iter().find_map(|event| match event {
        RawStrippedState::Pdu(raw) => serde_json::from_str::<CanonicalJsonObject>(raw.get())
            .ok()
            .filter(is_create),
        _ => None,
    });

    let Some(mut create) = create else {
        let stripped = state.iter().any(|event| match event {
            RawStrippedState::Stripped(raw) => {
                serde_json::from_str::<CanonicalJsonObject>(raw.json().get())
                    .is_ok_and(|json| is_create(&json))
            }
            _ => false,
        });

        return Ok(if stripped {
            StrippedCreateVerdict::NotPdu
        } else {
            StrippedCreateVerdict::Missing
        });
    };

    create.remove("unsigned");

    let bound = if v12_room_ids(room_version_id) {
        gen_event_id(&create, room_version_id)
            .ok()
            .and_then(|event_id| RoomId::parse(format!("!{}", event_id.localpart())).ok())
            .is_some_and(|expected| expected == room_id)
    } else {
        create
            .get("room_id")
            .and_then(CanonicalJsonValue::as_str)
            .is_some_and(|id| id == room_id.as_str())
    };

    if !bound {
        return Ok(StrippedCreateVerdict::WrongRoom);
    }

    if self
        .services
        .server_keys
        .verify_event(&create, Some(room_version_id))
        .await
        .is_err()
    {
        return Ok(StrippedCreateVerdict::BadSignature);
    }

    Ok(StrippedCreateVerdict::Valid)
}

fn is_create(json: &CanonicalJsonObject) -> bool {
    let field = |key| json.get(key).and_then(CanonicalJsonValue::as_str);

    field("type") == Some("m.room.create") && field("state_key") == Some("")
}

#[cfg(test)]
#[expect(deprecated)]
mod tests {
    use ruma::{
        RoomVersionId, api::federation::membership::RawStrippedState, events::StateEventType,
        room_id, serde::Raw, user_id,
    };
    use serde_json::{Value as JsonValue, json, value::RawValue as RawJsonValue};

    use super::{
        StrippedCreateVerdict, dedup_stripped_state, enforce_stripped_create, entry_json,
        into_client_stripped, is_legacy, occupies_member_cell, state_cell, v12_room_ids,
        without_member,
    };

    #[test]
    fn a_cells_first_pdu_wins_over_an_earlier_legacy_entry() {
        let deduped = dedup_stripped_state(vec![
            legacy(&create("@forged:example.org")),
            pdu(&create("@genuine:example.org")),
        ]);

        assert_eq!(senders(&deduped), ["@genuine:example.org"]);
    }

    #[test]
    fn a_cell_holding_no_pdu_keeps_its_first_legacy_entry() {
        let deduped = dedup_stripped_state(vec![
            legacy(&create("@first:example.org")),
            legacy(&create("@second:example.org")),
        ]);

        assert_eq!(senders(&deduped), ["@first:example.org"]);
    }

    #[test]
    fn distinct_cells_all_survive_in_order() {
        let deduped = dedup_stripped_state(vec![
            pdu(&create("@creator:example.org")),
            pdu(&member("@alice:example.org", "@alice:example.org")),
            pdu(&member("@bob:example.org", "@alice:example.org")),
        ]);

        assert_eq!(deduped.len(), 3);
        assert_eq!(
            state_cell(&deduped[0]).expect("a cell").0,
            StateEventType::RoomCreate
        );
    }

    #[test]
    fn an_entry_without_a_readable_cell_drops() {
        let deduped = dedup_stripped_state(vec![
            pdu(&json!({"sender": "@alice:example.org", "content": {}})),
            pdu(&create("@creator:example.org")),
        ]);

        assert_eq!(senders(&deduped), ["@creator:example.org"]);
    }

    #[test]
    fn the_invitees_membership_cell_never_survives() {
        let state = vec![
            pdu(&member("@invitee:example.org", "@forged:example.org")),
            pdu(&member("@other:example.org", "@alice:example.org")),
            pdu(&create("@creator:example.org")),
        ];

        let kept: Vec<_> = without_member(state, user_id!("@invitee:example.org")).collect();

        assert_eq!(
            senders(&kept),
            ["@alice:example.org", "@creator:example.org"]
        );
    }

    #[test]
    fn an_all_legacy_array_survives_with_every_cell_intact() {
        let name = json!({
            "type": "m.room.name",
            "state_key": "",
            "sender": "@creator:example.org",
            "content": {"name": "a room"},
        });

        let deduped = dedup_stripped_state(vec![
            legacy(&create("@creator:example.org")),
            legacy(&member("@alice:example.org", "@alice:example.org")),
            legacy(&name),
        ]);

        assert_eq!(deduped.len(), 3);
        assert!(deduped.iter().all(is_legacy));
        assert_eq!(
            senders(&deduped),
            [
                "@creator:example.org",
                "@alice:example.org",
                "@creator:example.org"
            ]
        );
    }

    #[test]
    fn a_cell_spelled_with_escapes_still_reads() {
        let escaped = raw_pdu(
            r#"{"type":"m.room.member","state_key":"@invitee:example.org",
               "sender":"@forged:example.org","content":{"membership":"invite"}}"#,
        );

        assert!(occupies_member_cell(
            &escaped,
            user_id!("@invitee:example.org")
        ));

        let mut kept = without_member(vec![escaped], user_id!("@invitee:example.org"));

        assert!(kept.next().is_none());
    }

    #[test]
    fn a_wrong_room_create_is_rejected_for_v12_even_unenforced() {
        assert!(enforce_stripped_create(
            StrippedCreateVerdict::WrongRoom,
            true,
            false
        ));
        assert!(!enforce_stripped_create(
            StrippedCreateVerdict::WrongRoom,
            false,
            false
        ));
    }

    #[test]
    fn a_missing_create_is_only_rejected_when_enforced() {
        for verdict in [
            StrippedCreateVerdict::Missing,
            StrippedCreateVerdict::NotPdu,
            StrippedCreateVerdict::BadSignature,
        ] {
            assert!(!enforce_stripped_create(verdict, true, false));
            assert!(enforce_stripped_create(verdict, false, true));
        }

        assert!(!enforce_stripped_create(
            StrippedCreateVerdict::Valid,
            true,
            true
        ));
    }

    #[test]
    fn only_v12_derives_room_ids_from_the_create_event() {
        assert!(!v12_room_ids(&RoomVersionId::V11));
        assert!(v12_room_ids(&RoomVersionId::V12));
    }

    #[test]
    fn a_pdu_is_reduced_to_the_client_shape() {
        let full = pdu(&json!({
            "type": "m.room.name",
            "state_key": "",
            "sender": "@creator:example.org",
            "room_id": "!room:example.org",
            "content": {"name": "a room"},
            "origin_server_ts": 1,
            "depth": 1,
            "auth_events": [],
            "prev_events": [],
            "hashes": {"sha256": "abc"},
            "signatures": {},
        }));

        let stripped = into_client_stripped(room_id!("!room:example.org"), full).expect("converts");
        let value: JsonValue = serde_json::from_str(stripped.json().get()).expect("json");

        assert_eq!(value["type"], "m.room.name");
        assert_eq!(value["sender"], "@creator:example.org");
        assert!(value.get("hashes").is_none());
        assert!(value.get("event_id").is_none());
    }

    fn legacy(event: &JsonValue) -> RawStrippedState {
        RawStrippedState::Stripped(Raw::new(event).expect("valid json").cast_unchecked())
    }

    fn raw_pdu(json: &str) -> RawStrippedState {
        RawStrippedState::Pdu(RawJsonValue::from_string(json.to_owned()).expect("valid json"))
    }

    fn pdu(event: &JsonValue) -> RawStrippedState {
        RawStrippedState::Pdu(
            Raw::<JsonValue>::new(event)
                .expect("valid json")
                .into_json(),
        )
    }

    fn create(sender: &str) -> JsonValue {
        json!({"type": "m.room.create", "state_key": "", "sender": sender, "content": {}})
    }

    fn member(user_id: &str, sender: &str) -> JsonValue {
        json!({
            "type": "m.room.member",
            "state_key": user_id,
            "sender": sender,
            "content": {"membership": "invite"},
        })
    }

    fn senders(state: &[RawStrippedState]) -> Vec<String> {
        state
            .iter()
            .map(|entry| {
                let value: JsonValue =
                    serde_json::from_str(entry_json(entry).expect("json")).expect("valid json");

                value["sender"].as_str().expect("a sender").to_owned()
            })
            .collect()
    }
}
