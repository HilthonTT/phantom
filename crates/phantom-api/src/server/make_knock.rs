use axum::extract::State;
use futures::TryFutureExt;
use phantom_core::{Err, Error, Result, at, debug_warn, matrix::pdu::PduBuilder};
use ruma::{
    api::{
        error::{ErrorKind, IncompatibleRoomVersionErrorData},
        federation::membership::prepare_knock_event,
    },
    events::room::member::{MembershipState, RoomMemberEventContent},
};

use super::{access::require_known_room, membership::reject_forbidden_room_server};
use crate::router::Ruma;

pub(crate) async fn create_knock_event_template_route(
    State(services): State<crate::router::State>,
    body: Ruma<prepare_knock_event::v1::Request>,
) -> Result<prepare_knock_event::v1::Response> {
    require_known_room(&services, &body.room_id, body.origin()).await?;

    if body.user_id.server_name() != body.origin() {
        return Err!(Request(BadJson(
            "Not allowed to knock on behalf of another server/user."
        )));
    }

    reject_forbidden_room_server(&services, body.origin(), &body.room_id)?;

    let room_version = services.rooms.state.get_room_version(&body.room_id).await?;

    if !body.ver.contains(&room_version) {
        return Err(incompatible_room_version(
            room_version,
            "Your homeserver does not support the features required to knock on this room.",
        ));
    }

    if !room_version
        .rules()
        .is_some_and(|rules| rules.authorization.knocking)
    {
        return Err(incompatible_room_version(
            room_version,
            "Room version does not support knocking.",
        ));
    }

    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    if let Ok(member) = services
        .rooms
        .state_accessor
        .get_member(&body.room_id, &body.user_id)
        .await
        && member.membership == MembershipState::Ban
    {
        debug_warn!(
            "Remote user {} is banned from {} but attempted to knock",
            body.user_id,
            body.room_id
        );

        return Err!(Request(Forbidden(
            "You cannot knock on a room you are banned from."
        )));
    }

    let pdu_json = services
        .rooms
        .timeline
        .create_hash_and_sign_event(
            PduBuilder::state(
                body.user_id.to_string(),
                &RoomMemberEventContent::new(MembershipState::Knock),
            ),
            &body.user_id,
            &body.room_id,
            &state_lock,
        )
        .map_ok(at!(1))
        .await?;

    drop(state_lock);

    let event = services
        .federation
        .format_pdu(pdu_json, Some(&room_version))
        .await;

    Ok(prepare_knock_event::v1::Response::new(room_version, event))
}

fn incompatible_room_version(room_version: ruma::RoomVersionId, message: &'static str) -> Error {
    Error::BadRequest(
        ErrorKind::IncompatibleRoomVersion(IncompatibleRoomVersionErrorData::new(room_version)),
        message,
    )
}
