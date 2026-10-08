pub(super) mod invite;
pub(super) mod make_join;
pub(super) mod make_knock;
pub(super) mod make_leave;
pub(super) mod restricted_join;
pub(super) mod send_join;
pub(super) mod send_knock;
pub(super) mod send_leave;

use phantom_core::{Err, Result, err, matrix::pdu::gen_event_id_canonical_json};
use phantom_service::{Services, ops::moderation::Restriction, rooms::timeline::RawPduId};
use ruma::{
    CanonicalJsonObject, EventId, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, RoomVersionId,
    ServerName,
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};
use serde::de::DeserializeOwned;
use serde_json::value::RawValue as RawJsonValue;

pub(super) struct IncomingMembership {
    pub(super) event_id: OwnedEventId,
    pub(super) value: CanonicalJsonObject,
    pub(super) content: RoomMemberEventContent,
    pub(super) sender: OwnedUserId,
}

pub(super) fn reject_forbidden_room_server(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
) -> Result {
    let Some(server) = room_id.server_name() else {
        return Ok(());
    };

    if services.moderation.forbids(server, Restriction::Federation) {
        return Err!(Request(Forbidden(warn!(
            "Server {origin} used room {room_id} whose server name is forbidden."
        ))));
    }

    Ok(())
}

pub(super) async fn parse_membership_event(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    pdu: &RawJsonValue,
    expected: MembershipState,
) -> Result<IncomingMembership> {
    let (event_id, value) = gen_event_id_canonical_json(pdu, room_version).map_err(|_| {
        err!(Request(BadJson(
            "Could not convert event to canonical JSON."
        )))
    })?;

    let event_room_id: OwnedRoomId = field(&value, "room_id")?;
    if event_room_id != room_id {
        return Err!(Request(BadJson(
            "Event room_id does not match request path room ID."
        )));
    }

    let event_type: StateEventType = field(&value, "type")?;
    if event_type != StateEventType::RoomMember {
        return Err!(Request(BadJson(
            "Only membership events are accepted on this endpoint."
        )));
    }

    let content: RoomMemberEventContent = field(&value, "content")?;
    if content.membership != expected {
        return Err!(Request(BadJson(
            "Only {expected} membership events are accepted on this endpoint."
        )));
    }

    let sender: OwnedUserId = field(&value, "sender")?;

    services
        .rooms
        .event_handler
        .acl_check(sender.server_name(), room_id)
        .await?;

    if sender.server_name() != origin {
        return Err!(Request(Forbidden(
            "Not allowed to act on behalf of another server."
        )));
    }

    let state_key: OwnedUserId = field(&value, "state_key")?;
    if state_key != sender {
        return Err!(Request(BadJson("State key does not match sender user.")));
    }

    Ok(IncomingMembership {
        event_id,
        value,
        content,
        sender,
    })
}

pub(super) async fn accept_timeline_event(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
    value: CanonicalJsonObject,
) -> Result<RawPduId> {
    let federation_lock = services
        .rooms
        .event_handler
        .mutex_federation
        .lock(room_id)
        .await;

    let pdu_id = services
        .rooms
        .event_handler
        .handle_incoming_pdu(origin, room_id, event_id, value, true)
        .await?
        .ok_or_else(|| err!(Request(Forbidden("Could not accept as timeline event."))))?;

    drop(federation_lock);

    Ok(pdu_id)
}

fn field<T: DeserializeOwned>(value: &CanonicalJsonObject, key: &str) -> Result<T> {
    let field = value
        .get(key)
        .ok_or_else(|| err!(Request(BadJson("Event is missing the {key} property."))))?;

    serde_json::from_value(field.clone().into())
        .map_err(|e| err!(Request(BadJson("Event has an invalid {key} property: {e}"))))
}
