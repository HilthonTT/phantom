use axum::extract::State;
use phantom_core::Result;
use ruma::api::federation::authorization::get_event_authorization;

use super::{
    access::check_event_in_room_access,
    federation_pdus::{auth_chain_ids, federation_pdus},
};
use crate::router::Ruma;

pub(crate) async fn get_event_authorization_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_event_authorization::v1::Request>,
) -> Result<get_event_authorization::v1::Response> {
    check_event_in_room_access(&services, body.origin(), &body.room_id, &body.event_id).await?;

    let room_version = services.rooms.state.get_room_version(&body.room_id).await?;
    let auth_chain_ids = auth_chain_ids(&services, &body.room_id, &body.event_id).await?;

    let auth_chain = federation_pdus(&services, auth_chain_ids, &room_version).await;

    Ok(get_event_authorization::v1::Response::new(auth_chain))
}
