use std::{borrow::Borrow, iter::once};

use axum::extract::State;
use phantom_core::Result;
use ruma::api::federation::authorization::get_event_authorization;

use super::{
    AccessCheck,
    access::require_event_in_room,
    federation_pdus::{auth_chain_ids, federation_pdus},
};
use crate::router::Ruma;

pub(crate) async fn get_event_authorization_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_event_authorization::v1::Request>,
) -> Result<get_event_authorization::v1::Response> {
    AccessCheck {
        services: &services,
        origin: body.origin(),
        room_id: &body.room_id,
        event_id: None,
    }
    .check()
    .await?;

    require_event_in_room(&services, &body.event_id, &body.room_id).await?;

    let room_version = services.rooms.state.get_room_version(&body.room_id).await?;
    let auth_chain_ids =
        auth_chain_ids(&services, &body.room_id, once(body.event_id.borrow())).await?;

    let auth_chain = federation_pdus(&services, auth_chain_ids, &room_version).await;

    Ok(get_event_authorization::v1::Response::new(auth_chain))
}
