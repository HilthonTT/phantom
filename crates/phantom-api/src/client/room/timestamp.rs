use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::client::room::get_event_by_timestamp::v1;

use crate::router::Ruma;

/// # `GET /_matrix/client/v1/rooms/{roomId}/timestamp_to_event`
///
/// Get the ID of the event closest to the given timestamp.
pub(crate) async fn get_event_by_timestamp_route(
    State(services): State<crate::router::State>,
    body: Ruma<v1::Request>,
) -> Result<v1::Response> {
    let sender_user = body.sender_user();
    let room_id = &body.room_id;

    if !services
        .rooms
        .state_accessor
        .user_can_see_state_events(sender_user, room_id)
        .await
    {
        return Err!(Request(Forbidden(
            "You don't have permission to view this room."
        )));
    }

    // Only events this server holds are searched; there is no federation
    // fallback for rooms with no local event near the timestamp.
    let (origin_server_ts, event_id) = services
        .rooms
        .timeline
        .get_event_id_near_ts(room_id, body.ts, body.dir)
        .await?;

    // An event with no recorded state is visible, matching the visibility check.
    if !services
        .rooms
        .state_accessor
        .user_can_see_event(sender_user, room_id, &event_id)
        .await
    {
        return Err!(Request(Forbidden(
            "You don't have permission to view this event."
        )));
    }

    Ok(v1::Response::new(event_id, origin_server_ts))
}
