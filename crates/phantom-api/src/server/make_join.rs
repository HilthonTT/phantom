use axum::extract::State;
use futures::{StreamExt, TryFutureExt, pin_mut};
use phantom_core::{Err, Error, Result, at, err, matrix::pdu::PduBuilder};
use ruma::{
    api::{
        error::{ErrorKind, IncompatibleRoomVersionErrorData},
        federation::membership::prepare_join_event,
    },
    events::room::member::{MembershipState, RoomMemberEventContent},
};

use super::{
    access::require_known_room, membership::reject_forbidden_room_server,
    restricted_join::requires_authorising_user,
};
use crate::router::Ruma;

pub(crate) async fn create_join_event_template_route(
    State(services): State<crate::router::State>,
    body: Ruma<prepare_join_event::v1::Request>,
) -> Result<prepare_join_event::v1::Response> {
    require_known_room(&services, &body.room_id, body.origin()).await?;

    if body.user_id.server_name() != body.origin() {
        return Err!(Request(BadJson(
            "Not allowed to join on behalf of another server/user."
        )));
    }

    reject_forbidden_room_server(&services, body.origin(), &body.room_id)?;

    let room_version = services.rooms.state.get_room_version(&body.room_id).await?;

    if !body.ver.contains(&room_version) {
        return Err(Error::BadRequest(
            ErrorKind::IncompatibleRoomVersion(IncompatibleRoomVersionErrorData::new(room_version)),
            "Room version not supported.",
        ));
    }

    let rules = room_version.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}."
        )))
    })?;

    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    let join_authorized_via_users_server =
        if requires_authorising_user(&services, &body.user_id, &body.room_id, &rules).await? {
            let authorisers = services
                .rooms
                .state_cache
                .local_users_in_room(&body.room_id)
                .filter(|user| {
                    services.rooms.state_accessor.user_can_invite(
                        &body.room_id,
                        user,
                        &body.user_id,
                        &state_lock,
                    )
                })
                .map(ToOwned::to_owned);

            pin_mut!(authorisers);
            let Some(authoriser) = authorisers.next().await else {
                return Err!(Request(UnableToGrantJoin(
                    "No user on this server is able to assist in joining."
                )));
            };

            Some(authoriser)
        } else {
            None
        };

    let mut content = RoomMemberEventContent::new(MembershipState::Join);
    content.join_authorized_via_users_server = join_authorized_via_users_server;

    let pdu_json = services
        .rooms
        .timeline
        .create_hash_and_sign_event(
            PduBuilder::state(body.user_id.to_string(), &content),
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

    let mut response = prepare_join_event::v1::Response::new(event);
    response.room_version = Some(room_version);

    Ok(response)
}
