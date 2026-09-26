use axum::extract::State;
use futures::{FutureExt, future::try_join};
use phantom_core::{Err, Result, err, matrix::pdu::PduEvent};
use ruma::{
    api::federation::membership::create_knock_event, events::room::member::MembershipState,
};

use super::{
    access::require_known_room,
    membership::{accept_timeline_event, parse_membership_event, reject_forbidden_room_server},
};
use crate::router::Ruma;

pub(crate) async fn create_knock_event_v1_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_knock_event::v1::Request>,
) -> Result<create_knock_event::v1::Response> {
    let room_id = &body.room_id;
    let origin = body.origin();

    reject_forbidden_room_server(&services, origin, room_id)?;

    services.sending.notify_peer_alive(origin).await;

    require_known_room(&services, room_id, origin).await?;

    let room_version = services.rooms.state.get_room_version(room_id).await?;

    if !room_version
        .rules()
        .is_some_and(|rules| rules.authorization.knocking)
    {
        return Err!(Request(Forbidden(
            "Room version does not support knocking."
        )));
    }

    let knock = parse_membership_event(
        &services,
        origin,
        room_id,
        &room_version,
        &body.pdu,
        MembershipState::Knock,
    )
    .await?;

    let mut pdu_json = knock.value.clone();
    pdu_json.insert("event_id".to_owned(), knock.event_id.as_str().into());

    let pdu: PduEvent = serde_json::from_value(serde_json::to_value(&pdu_json)?)
        .map_err(|e| err!(Request(InvalidParam("Invalid knock event PDU: {e}"))))?;

    let pdu_id = accept_timeline_event(
        &services,
        origin,
        room_id,
        &knock.event_id,
        knock.value.clone(),
    )
    .await?;

    let broadcast = services.sending.send_pdu_room(room_id, &pdu_id);
    let knock_room_state = services
        .rooms
        .membership
        .summary_pdus(&pdu, &knock.value, &room_version)
        .map(Ok);

    let (knock_room_state, ()) = try_join(knock_room_state, broadcast).await?;

    Ok(create_knock_event::v1::Response::new(knock_room_state))
}
