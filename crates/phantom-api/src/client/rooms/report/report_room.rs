use axum::extract::State;
use phantom_core::{Err, Result, info};
use ruma::api::client::room::report_room;

use super::{REASON_MAX_LEN, Reported, send_report};
use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/rooms/{roomId}/report`
///
/// Reports an abusive room to homeserver admins
#[tracing::instrument(skip_all, fields(%client), name = "report_room")]
pub(crate) async fn report_room_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<report_room::v3::Request>,
) -> Result<report_room::v3::Response> {
    let sender_user = body.sender_user();

    info!(
        "Received room report by user {sender_user} for room {} with reason: \"{}\"",
        body.room_id, body.reason,
    );

    if body.reason.len() > REASON_MAX_LEN {
        return Err!(Request(InvalidParam(
            "Reason too long, should be {REASON_MAX_LEN} characters or fewer"
        )));
    }

    if !services
        .rooms
        .state_cache
        .server_in_room(&services.server.name, &body.room_id)
        .await
    {
        return Err!(Request(NotFound(
            "Room does not exist to us, no local users have joined at all"
        )));
    }

    send_report(
        &services,
        sender_user,
        Reported {
            room_id: Some(&body.room_id),
            event_id: None,
            user_id: None,
        },
        &body.reason,
        &format!(
            "@room Room report received from {}\nReport Reason: {}\n\nRoom ID: {}",
            sender_user, body.reason, body.room_id,
        ),
    )
    .await;

    Ok(report_room::v3::Response::new())
}
