use axum::extract::State;
use futures::FutureExt;
use phantom_core::Result;
use ruma::{
    api::federation::membership::create_leave_event, events::room::member::MembershipState,
};

use super::{
    access::require_known_room,
    membership::{accept_timeline_event, parse_membership_event},
};
use crate::router::Ruma;

pub(crate) async fn create_leave_event_v2_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_leave_event::v2::Request>,
) -> Result<create_leave_event::v2::Response> {
    let room_id = &body.room_id;
    let origin = body.origin();

    services.sending.notify_peer_alive(origin).await;

    require_known_room(&services, room_id, origin).await?;

    let room_version = services.rooms.state.get_room_version(room_id).await?;
    let leave = parse_membership_event(
        &services,
        origin,
        room_id,
        &room_version,
        &body.pdu,
        MembershipState::Leave,
    )
    .await?;

    let pdu_id =
        accept_timeline_event(&services, origin, room_id, &leave.event_id, leave.value).await?;

    services
        .sending
        .send_pdu_room(room_id, &pdu_id)
        .boxed()
        .await?;

    Ok(create_leave_event::v2::Response::new())
}
