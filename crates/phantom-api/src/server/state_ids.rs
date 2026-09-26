use std::{borrow::Borrow, iter::once};

use axum::extract::State;
use futures::StreamExt;
use phantom_core::{Result, at, err};
use ruma::{OwnedEventId, api::federation::event::get_room_state_ids};

use super::{AccessCheck, access::require_event_in_room, federation_pdus::auth_chain_ids};
use crate::router::Ruma;

pub(crate) async fn get_room_state_ids_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_state_ids::v1::Request>,
) -> Result<get_room_state_ids::v1::Response> {
    AccessCheck {
        services: &services,
        origin: body.origin(),
        room_id: &body.room_id,
        event_id: None,
    }
    .check()
    .await?;

    require_event_in_room(&services, &body.event_id, &body.room_id).await?;

    let shortstatehash = services
        .rooms
        .state_accessor
        .pdu_shortstatehash(&body.event_id)
        .await
        .map_err(|_| err!(Request(NotFound("PDU state not found."))))?;

    let auth_chain_ids =
        auth_chain_ids(&services, &body.room_id, once(body.event_id.borrow())).await?;

    let pdu_ids: Vec<OwnedEventId> = services
        .rooms
        .state_accessor
        .state_full_ids(shortstatehash)
        .map(at!(1))
        .collect()
        .await;

    Ok(get_room_state_ids::v1::Response::new(
        auth_chain_ids,
        pdu_ids,
    ))
}
