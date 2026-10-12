use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedEventId, OwnedRoomId, RoomId,
    RoomVersionId,
};
use serde_json::value::RawValue as RawJsonValue;

use crate::{Result, err};

pub fn gen_event_id_canonical_json(
    pdu: &RawJsonValue,
    room_version_id: &RoomVersionId,
) -> Result<(OwnedEventId, CanonicalJsonObject)> {
    let value: CanonicalJsonObject = serde_json::from_str(pdu.get()).map_err(|e| {
        err!(BadServerResponse(warn!(
            "Error parsing incoming event: {e:?}"
        )))
    })?;

    let event_id = gen_event_id(&value, room_version_id)?;

    Ok((event_id, value))
}

pub fn gen_event_id(
    value: &CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> Result<OwnedEventId> {
    let rules = room_version_id.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version_id}"
        )))
    })?;

    let reference_hash = ruma::signatures::reference_hash(value, &rules)?;
    let event_id: OwnedEventId = format!("${reference_hash}").try_into()?;

    Ok(event_id)
}

/// The room a PDU's JSON belongs to. Since room version 12 a create event
/// carries no `room_id`: its room ID is its own event ID under a `!` sigil,
/// taken from `event_id` when present and computed otherwise.
#[must_use]
pub fn room_id_of_pdu_json(value: &CanonicalJsonObject) -> Option<OwnedRoomId> {
    if let Some(room_id) = value.get("room_id") {
        return room_id.as_str().and_then(|id| RoomId::parse(id).ok());
    }

    if value.get("type").and_then(CanonicalJsonValue::as_str) != Some("m.room.create") {
        return None;
    }

    let event_id = match value.get("event_id").and_then(CanonicalJsonValue::as_str) {
        Some(event_id) => EventId::parse(event_id).ok()?,
        None => gen_event_id(value, &RoomVersionId::V12).ok()?,
    };

    room_id_of_create_event_id(&event_id)
}

/// The room ID derived from a room-version-12 create event's ID.
#[must_use]
pub fn room_id_of_create_event_id(event_id: &EventId) -> Option<OwnedRoomId> {
    let opaque = event_id.as_str().strip_prefix('$')?;
    RoomId::parse(format!("!{opaque}")).ok()
}

/// Whether the room version derives room IDs from create events, so that a
/// create event's JSON must carry no `room_id` (room version 12 onwards).
#[must_use]
pub fn create_omits_room_id(room_version_id: &RoomVersionId) -> bool {
    room_version_id
        .rules()
        .is_some_and(|rules| !rules.event_format.require_room_create_room_id)
}

/// Gives a room-version-12 create event's JSON the `room_id` its event ID
/// implies, so it stores and parses like any other PDU. Call it only after the
/// event's hashes and signatures were checked, which cover the JSON without it.
pub fn insert_derived_room_id(value: &mut CanonicalJsonObject, event_id: &EventId) {
    if value.contains_key("room_id")
        || value.get("type").and_then(CanonicalJsonValue::as_str) != Some("m.room.create")
    {
        return;
    }

    if let Some(room_id) = room_id_of_create_event_id(event_id) {
        value.insert(
            "room_id".to_owned(),
            CanonicalJsonValue::String(room_id.into()),
        );
    }
}
