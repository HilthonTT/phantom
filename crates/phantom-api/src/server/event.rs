use axum::extract::State;
use futures::future::try_join;
use phantom_core::{Result, err};
use ruma::{MilliSecondsSinceUnixEpoch, OwnedRoomId, api::federation::event::get_event};

use super::AccessCheck;
use crate::router::Ruma;

pub(crate) async fn get_event_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_event::v1::Request>,
) -> Result<get_event::v1::Response> {
    let event = services
        .rooms
        .timeline
        .get_pdu_json(&body.event_id)
        .await
        .map_err(|_| err!(Request(NotFound("Event not found."))))?;

    let room_id: OwnedRoomId = event
        .get("room_id")
        .and_then(|val| val.as_str())
        .ok_or_else(|| err!(Database("Invalid event in database.")))?
        .try_into()
        .map_err(|_| err!(Database("Invalid room_id in event in database.")))?;

    let access_check = AccessCheck {
        services: &services,
        origin: body.origin(),
        room_id: &room_id,
        event_id: Some(&body.event_id),
    };

    let pdu = async { Ok(services.federation.format_pdu(event, None).await) };
    let ((), pdu) = try_join(access_check.check(), pdu).await?;

    Ok(get_event::v1::Response::new(
        services.server_state.server_name().to_owned(),
        MilliSecondsSinceUnixEpoch::now(),
        pdu,
    ))
}
