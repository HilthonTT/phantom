use std::iter::once;

use futures::{StreamExt, TryStreamExt, future::ready, stream::FuturesOrdered};
use phantom_core::{Result, at, debug_error, err};
use phantom_service::Services;
use ruma::{EventId, OwnedEventId, RoomId, RoomVersionId};
use serde_json::value::RawValue as RawJsonValue;

pub(super) async fn federation_pdus<I>(
    services: &Services,
    event_ids: I,
    room_version: &RoomVersionId,
) -> Vec<Box<RawJsonValue>>
where
    I: IntoIterator<Item = OwnedEventId>,
{
    event_ids
        .into_iter()
        .map(|event_id| federation_pdu(services, event_id, room_version))
        .collect::<FuturesOrdered<_>>()
        .filter_map(|pdu| ready(pdu.ok()))
        .collect()
        .await
}

async fn federation_pdu(
    services: &Services,
    event_id: OwnedEventId,
    room_version: &RoomVersionId,
) -> Result<Box<RawJsonValue>> {
    let pdu = services
        .rooms
        .timeline
        .get_pdu_json(&event_id)
        .await
        .inspect_err(|e| debug_error!("Event {event_id} not found: {e}"))?;

    Ok(services
        .federation
        .format_pdu(pdu, Some(room_version))
        .await)
}

pub(super) async fn auth_chain_ids(
    services: &Services,
    room_id: &RoomId,
    event_id: &EventId,
) -> Result<Vec<OwnedEventId>> {
    services
        .rooms
        .auth_chain
        .event_ids_iter(room_id, once(event_id))
        .try_collect()
        .await
}

/// IDs of the full room state at `event_id`.
pub(super) async fn state_ids_at(
    services: &Services,
    event_id: &EventId,
) -> Result<Vec<OwnedEventId>> {
    let shortstatehash = services
        .rooms
        .state_accessor
        .pdu_shortstatehash(event_id)
        .await
        .map_err(|_| err!(Request(NotFound("PDU state not found."))))?;

    Ok(services
        .rooms
        .state_accessor
        .state_full_ids(shortstatehash)
        .map(at!(1))
        .collect()
        .await)
}
