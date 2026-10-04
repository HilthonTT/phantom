use axum::extract::State;
use futures::TryFutureExt;
use phantom_core::{Err, Result, err};
use ruma::api::client::room::get_room_event;

use crate::{
    client::{annotate_membership, is_ignored_pdu, utils::sender_ignored},
    router::Ruma,
};

/// # `GET /_matrix/client/r0/rooms/{roomId}/event/{eventId}`
///
/// Gets a single event.
pub(crate) async fn get_room_event_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_event::v3::Request>,
) -> Result<get_room_event::v3::Response> {
    let sender_user = body.sender_user();
    let event_id = &body.event_id;
    let room_id = &body.room_id;

    let mut event = services
        .rooms
        .timeline
        .get_pdu(event_id)
        .map_err(|_| err!(Request(NotFound("Event {} not found.", event_id))))
        .await?;

    if event.room_id != *room_id
        || !services
            .rooms
            .state_accessor
            .user_can_see_event(sender_user, room_id, event_id)
            .await
    {
        return Err!(Request(NotFound("Event not found.")));
    }

    if is_ignored_pdu(&services, &event, sender_user).await {
        return Err(sender_ignored(&event.sender));
    }

    debug_assert!(
        event.event_id == *event_id,
        "Fetched PDU must match requested"
    );

    event.add_age().ok();

    let encrypted = services
        .rooms
        .state_accessor
        .is_encrypted_room(room_id)
        .await;

    annotate_membership(&services, &mut event, sender_user, encrypted).await;

    Ok(get_room_event::v3::Response::new(event.into_room_event()))
}
