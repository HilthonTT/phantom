use axum::extract::State;
use phantom_core::{Err, Result, matrix::pdu::PduBuilder, warn};
use ruma::{api::client::redact::redact_event, events::room::redaction::RoomRedactionEventContent};

use crate::router::Ruma;

/// # `PUT /_matrix/client/r0/rooms/{roomId}/redact/{eventId}/{txnId}`
///
/// Tries to send a redaction event into the room.
///
/// - TODO: Handle txn id
pub(crate) async fn redact_event_route(
    State(services): State<crate::router::State>,
    body: Ruma<redact_event::v3::Request>,
) -> Result<redact_event::v3::Response> {
    let sender_user = body.sender_user();

    if services.config.client.disable_local_redactions
        && !services.admin.user_is_admin(sender_user).await
    {
        warn!(
            message = format_args!("Local redactions are disabled, non-admin user attempted to redact an event"),
            %sender_user,
            event_id = %body.event_id
        );
        return Err!(Request(Forbidden(
            "Redactions are disabled on this server."
        )));
    }

    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    let mut redaction = RoomRedactionEventContent::new_v11(body.event_id.clone());
    redaction.reason = body.reason.clone();

    let event_id = services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                redacts: Some(body.event_id.clone()),
                ..PduBuilder::timeline(&redaction)
            },
            sender_user,
            &body.room_id,
            &state_lock,
        )
        .await?;

    drop(state_lock);

    Ok(redact_event::v3::Response::new(event_id))
}
