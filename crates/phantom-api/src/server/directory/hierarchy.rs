use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::federation::space::get_hierarchy;

use crate::router::Ruma;

pub(crate) async fn get_hierarchy_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_hierarchy::v1::Request>,
) -> Result<get_hierarchy::v1::Response> {
    if !services.rooms.metadata.exists(&body.room_id).await {
        return Err!(Request(NotFound("Room does not exist.")));
    }

    services
        .rooms
        .spaces
        .federation_hierarchy(&body.room_id, body.origin(), body.suggested_only)
        .await
}
