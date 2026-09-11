use phantom_core::{Result, err, implement, matrix::pdu::gen_event_id_canonical_json};
use ruma::{CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedRoomId, RoomId};
use serde_json::value::RawValue as RawJsonValue;

use super::Service;

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

    let room_version_id = self.services.state.get_room_version(&room_id).await?;

    let (event_id, value) = gen_event_id_canonical_json(pdu, &room_version_id)?;

    Ok((event_id, value, room_id))
}
