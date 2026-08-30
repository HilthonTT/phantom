//! Signing with this server's key.

use phantom_core::{Err, Result, implement};
use ruma::{CanonicalJsonObject, RoomVersionId};

/// Signs `object` as this server, adding to its `signatures`.
#[implement(super::Service)]
pub fn sign_json(&self, object: &mut CanonicalJsonObject) -> Result {
    use ruma::signatures::sign_json;

    let server_name = self.services.server_state.server_name().as_str();

    sign_json(server_name, self.keypair(), object).map_err(Into::into)
}

/// Adds the content hash to `object` and then signs it as this server.
///
/// What is signed is the event with everything a redaction would remove taken
/// out, so that redacting it later does not invalidate the signature. Which
/// fields those are is a property of the room version, which is why this
/// needs one and [`Self::sign_json`] does not.
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
