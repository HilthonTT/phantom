use axum::extract::State;
use phantom_core::Result;
use ruma::api::federation::event::get_room_state;

use super::{
    access::check_event_in_room_access,
    federation_pdus::{auth_chain_ids, federation_pdus, state_ids_at},
};
use crate::router::Ruma;

pub(crate) async fn get_room_state_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_state::v1::Request>,
) -> Result<get_room_state::v1::Response> {
    check_event_in_room_access(&services, body.origin(), &body.room_id, &body.event_id).await?;

    let room_version = services.rooms.state.get_room_version(&body.room_id).await?;
    let state_ids = state_ids_at(&services, &body.event_id).await?;
    let auth_chain_ids = auth_chain_ids(&services, &body.room_id, &body.event_id).await?;

    let auth_chain = federation_pdus(&services, auth_chain_ids, &room_version).await;
    let pdus = federation_pdus(&services, state_ids, &room_version).await;

    Ok(get_room_state::v1::Response::new(auth_chain, pdus))
}
