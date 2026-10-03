use axum::extract::State;
use futures::{TryFutureExt, future::join, pin_mut};
use ruma::api::client::room::get_room_event;
use phantom_core::{Err, matrix::Event, Result, err, result::IsErrOr, bool::BoolExt, future::BoolExt as FutureBoolExt, future::TryExt as TryFutureExtExt, future::OptionFutureExt, matrix::Pdu};

use crate::client::utils::sender_ignored;
use crate::{
    router::Ruma,
    client::{annotate_membership, is_ignored_pdu},
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

    let event = services
        .rooms
        .timeline
        .get_pdu(event_id)
        .map_err(|_| err!(Request(NotFound("Event {} not found.", event_id))));

    let retained_event = body
        .include_unredacted_content
        .then_async(async || {
            let is_admin = services.admin.user_is_admin(sender_user);

            let can_redact = services
                .config
                .client.allow_room_admins_to_request_unredacted_events
                .then_async(|| {
                    services
                        .rooms
                        .state_accessor
                        .get_power_levels(room_id)
                        .map_ok_or(false, |power_levels| {
                            power_levels.for_user(sender_user) >= power_levels.redact
                        })
                })
                .unwrap_or(false);

            pin_mut!(is_admin, can_redact);

            if is_admin.or(can_redact).await {
                services
                    .rooms
                    .retention
                    .get_original_pdu(event_id)
                    .await
                    .map_err(|_| err!(Request(NotFound("Event {} not found.", event_id))))
            } else {
                Err!(Request(Forbidden("You are not allowed to see the original event")))
            }
        });

    let (event, retained_event) = join(event, retained_event).await;

    let event: Result<Pdu> = retained_event
        .filter(|_| event.as_ref().is_err_or(Event::is_redacted))
        .unwrap_or(event);

    let mut event = event?;

    if event.room_id() != room_id
        || !services
            .rooms
            .state_accessor
            .user_can_see_event(sender_user, &event)
            .await
    {
        return Err!(Request(NotFound("Event not found.")));
    }

    if is_ignored_pdu(&services, &event, body.sender_user()).await {
        return Err(sender_ignored(event.sender()));
    }

    debug_assert!(event.event_id() == event_id, "Fetched PDU must match requested");

    event.add_age().ok();

    let encrypted = services
        .rooms
        .state_accessor
        .is_encrypted_room(room_id)
        .await;

    annotate_membership(&services, &mut event, sender_user, encrypted).await;

    let event = services
        .rooms
        .pdu_metadata
        .bundle_aggregations(sender_user, event)
        .await;

    Ok(get_room_event::v3::Response::new(event.into_format()))
}
