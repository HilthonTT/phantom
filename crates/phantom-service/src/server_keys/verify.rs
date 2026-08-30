//! Verifying that an event is signed by the servers it claims.

use phantom_core::{Err, Result, implement, matrix::pdu::gen_event_id_canonical_json};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, RoomVersionId, signatures::Verified,
};
use serde_json::value::RawValue as RawJsonValue;

/// The room version assumed for an object that is not an event in a room —
/// a federation request's signed body, most of all — where the rules that
/// differ between versions do not apply.
const DEFAULT_ROOM_VERSION: RoomVersionId = RoomVersionId::V11;

/// Derives an incoming PDU's event id, verifies it, and writes the id back
/// into the event.
///
/// An event arrives over federation without its id: the id *is* the hash of
/// the event, so it is computed here rather than trusted. Missing keys are
/// fetched, which means this can go out to the network.
#[implement(super::Service)]
pub async fn validate_and_add_event_id(
    &self,
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
) -> Result<(OwnedEventId, CanonicalJsonObject)> {
    let (event_id, mut value) = gen_event_id_canonical_json(pdu, room_version)?;

    if let Err(e) = self.verify_event(&value, Some(room_version)).await {
        return Err!(BadServerResponse(debug_error!(
            "Event {event_id} failed verification: {e:?}"
        )));
    }

    value.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok((event_id, value))
}

/// [`Self::validate_and_add_event_id`] without going out to the network.
///
/// For the paths handling a batch of events that has already had its keys
/// acquired in one go by [`Self::acquire_events_pubkeys`]: an event whose
/// keys are still missing after that is rejected rather than fetched one at a
/// time.
#[implement(super::Service)]
pub async fn validate_and_add_event_id_no_fetch(
    &self,
    pdu: &RawJsonValue,
    room_version: &RoomVersionId,
) -> Result<(OwnedEventId, CanonicalJsonObject)> {
    let (event_id, mut value) = gen_event_id_canonical_json(pdu, room_version)?;

    if !self.required_keys_exist(&value, room_version).await {
        return Err!(BadServerResponse(debug_warn!(
            "Event {event_id} cannot be verified: missing keys."
        )));
    }

    if let Err(e) = self.verify_event(&value, Some(room_version)).await {
        return Err!(BadServerResponse(debug_error!(
            "Event {event_id} failed verification: {e:?}"
        )));
    }

    value.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok((event_id, value))
}

#[implement(super::Service)]
pub async fn verify_event(
    &self,
    event: &CanonicalJsonObject,
    room_version: Option<&RoomVersionId>,
) -> Result<Verified> {
    let room_version = room_version.unwrap_or(&DEFAULT_ROOM_VERSION);
    let Some(rules) = room_version.rules() else {
        return Err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}"
        )));
    };

    let keys = self.get_event_keys(event, room_version).await?;

    ruma::signatures::verify_event(&keys, event, &rules).map_err(Into::into)
}

#[implement(super::Service)]
pub async fn verify_json(
    &self,
    event: &CanonicalJsonObject,
    room_version: Option<&RoomVersionId>,
) -> Result {
    let room_version = room_version.unwrap_or(&DEFAULT_ROOM_VERSION);
    let keys = self.get_event_keys(event, room_version).await?;

    ruma::signatures::verify_json(&keys, event).map_err(Into::into)
}
