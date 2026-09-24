use phantom_core::{Err, Result, implement, matrix::pdu::gen_event_id};
use ruma::{CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, RoomVersionId};

#[implement(super::Service)]
pub fn sign_json(&self, object: &mut CanonicalJsonObject) -> Result {
    use ruma::signatures::sign_json;

    let server_name = self.services.server_state.server_name().as_str();

    sign_json(server_name, self.keypair(), object).map_err(Into::into)
}

#[implement(super::Service)]
pub fn hash_and_sign_event(
    &self,
    object: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Result {
    use ruma::signatures::hash_and_sign_event;

    let Some(rules) = room_version.rules() else {
        return Err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}"
        )));
    };

    let server_name = self.services.server_state.server_name().as_str();

    hash_and_sign_event(server_name, self.keypair(), object, &rules.redaction).map_err(Into::into)
}

#[implement(super::Service)]
pub fn gen_id_hash_and_sign_event(
    &self,
    object: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Result<OwnedEventId> {
    let Some(rules) = room_version.rules() else {
        return Err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}"
        )));
    };

    object.remove("event_id");

    if rules.event_format.require_event_id {
        let event_id = gen_event_id(object, room_version)?;
        object.insert(
            "event_id".into(),
            CanonicalJsonValue::String(event_id.as_str().into()),
        );
        self.hash_and_sign_event(object, room_version)?;

        return Ok(event_id);
    }

    self.hash_and_sign_event(object, room_version)?;

    let event_id = gen_event_id(object, room_version)?;
    object.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok(event_id)
}
