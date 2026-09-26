use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::federation::event::get_event_by_timestamp::v1;

use super::AccessCheck;
use crate::router::Ruma;

pub(crate) async fn get_event_by_timestamp_route(
    State(services): State<crate::router::State>,
    body: Ruma<v1::Request>,
) -> Result<v1::Response> {
    let origin = body.origin();
    let room_id = &body.room_id;

    AccessCheck {
        services: &services,
        origin,
        room_id,
        event_id: None,
    }
    .check()
    .await?;

    let (origin_server_ts, event_id) = services
        .rooms
        .timeline
        .get_event_id_near_ts(room_id, body.ts, body.dir)
        .await?;

    if !services
        .rooms
        .state_accessor
        .server_can_see_event(origin, room_id, &event_id)
        .await
    {
        return Err!(Request(Forbidden(
            "Server is not allowed to see this event"
        )));
    }

    Ok(v1::Response::new(event_id, origin_server_ts))
}
