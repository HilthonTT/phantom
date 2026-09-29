use axum::extract::State;
use phantom_core::Result;
use ruma::api::federation::event::get_room_state_ids;

use super::{
    access::check_event_in_room_access,
    federation_pdus::{auth_chain_ids, state_ids_at},
};
use crate::router::Ruma;

pub(crate) async fn get_room_state_ids_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
    check_event_in_room_access(&services, body.origin(), &body.room_id, &body.event_id).await?;

    let auth_chain_ids = auth_chain_ids(&services, &body.room_id, &body.event_id).await?;
    let pdu_ids = state_ids_at(&services, &body.event_id).await?;

    Ok(get_room_state_ids::v1::Response::new(
        auth_chain_ids,
        pdu_ids,
    ))
}
