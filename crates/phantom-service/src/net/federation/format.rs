use phantom_core::{
    implement,
    matrix::pdu::{create_omits_room_id, room_id_of_pdu_json},
};
use ruma::{CanonicalJsonObject, CanonicalJsonValue, RoomVersionId};
use serde_json::value::{RawValue as RawJsonValue, to_raw_value};

#[implement(super::Service)]
pub async fn format_pdu(
    &self,
    pdu_json: CanonicalJsonObject,
    room_version: Option<&RoomVersionId>,
) -> Box<RawJsonValue> {
    if let Some(room_version) = room_version {
        return outgoing_pdu(pdu_json, room_version);
    }

    let room_version = match room_id_of_pdu_json(&pdu_json) {
        Some(room_id) => self.services.state.get_room_version(&room_id).await.ok(),
        None => None,
    };

    match room_version {
        Some(room_version) => outgoing_pdu(pdu_json, &room_version),
        None => outgoing_pdu_without_event_id(pdu_json),
    }
}

#[must_use]
pub fn outgoing_pdu(
    pdu_json: CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Box<RawJsonValue> {
    let keeps_event_id = room_version
        .rules()
        .is_some_and(|rules| rules.event_format.require_event_id);

    let pdu_json = strip_derived_room_id(pdu_json, room_version);

    if keeps_event_id {
        serialize(strip_transaction_id(pdu_json))
    } else {
        outgoing_pdu_without_event_id(pdu_json)
    }
}

/// A room-version-12 create event is stored with the room_id its ID implies,
/// but its signed form, the one other servers hash, has none.
fn strip_derived_room_id(
    mut pdu_json: CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> CanonicalJsonObject {
    if create_omits_room_id(room_version)
        && pdu_json.get("type").and_then(CanonicalJsonValue::as_str) == Some("m.room.create")
    {
        pdu_json.remove("room_id");
    }

    pdu_json
}

fn outgoing_pdu_without_event_id(pdu_json: CanonicalJsonObject) -> Box<RawJsonValue> {
    let mut pdu_json = strip_transaction_id(pdu_json);
    pdu_json.remove("event_id");

    serialize(pdu_json)
}

fn strip_transaction_id(mut pdu_json: CanonicalJsonObject) -> CanonicalJsonObject {
    if let Some(unsigned) = pdu_json
        .get_mut("unsigned")
        .and_then(CanonicalJsonValue::as_object_mut)
    {
        unsigned.remove("transaction_id");
    }

    pdu_json
}

fn serialize(pdu_json: CanonicalJsonObject) -> Box<RawJsonValue> {
    to_raw_value(&pdu_json).expect("CanonicalJson is valid serde_json::Value")
}
