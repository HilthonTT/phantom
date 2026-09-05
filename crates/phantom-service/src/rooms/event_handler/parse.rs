//! Reading an event off the wire.
//!
//! What arrives is JSON with no event id in it, because in Matrix an event's
//! id *is* a hash of the event and is therefore computed rather than sent. The
//! room has to be identified first, since which fields go into that hash
//! depends on the room version, which is a property of the room and not of the
//! event.

use phantom_core::{Result, err, implement, matrix::pdu::gen_event_id_canonical_json};
use ruma::{CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedRoomId, RoomId};
use serde_json::value::RawValue as RawJsonValue;

use super::Service;

/// Turns one PDU from a federation transaction into an id, its canonical form,
/// and the room it claims to belong to.
///
/// The room is returned rather than taken as an argument because a transaction
/// carries events for any number of rooms and does not say which is which; the
/// event's own `room_id` is the only thing that does.
#[implement(Service)]
pub async fn parse_incoming_pdu(
    &self,
    pdu: &RawJsonValue,
) -> Result<(OwnedEventId, CanonicalJsonObject, OwnedRoomId)> {
    let value = serde_json::from_str::<CanonicalJsonObject>(pdu.get())
        .map_err(|e| err!(Request(InvalidParam("Event is not a JSON object: {e}"))))?;

    let room_id = value
        .get("room_id")
        .and_then(CanonicalJsonValue::as_str)
        .ok_or_else(|| err!(Request(InvalidParam("Event has no room_id."))))?;

    let room_id = RoomId::parse(room_id)
        .map_err(|e| err!(Request(InvalidParam("Event has an invalid room_id: {e}"))))?;

    // The room version is needed before the id can be computed, and it is only
    // knowable for a room this server is in. An event for anywhere else cannot
    // be parsed, let alone handled.
    let room_version_id = self.services.state.get_room_version(&room_id).await?;

    let (event_id, value) = gen_event_id_canonical_json(pdu, &room_version_id)?;

    Ok((event_id, value, room_id))
}
