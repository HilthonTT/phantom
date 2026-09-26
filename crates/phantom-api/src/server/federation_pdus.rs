use std::{fmt::Debug, pin::pin};

use futures::{StreamExt, stream::FuturesOrdered};
use phantom_core::{Result, debug_error};
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
    let mut pdus = FuturesOrdered::new();
    for event_id in event_ids {
        pdus.push_back(federation_pdu(services, event_id, room_version));
    }

    let mut formatted = Vec::with_capacity(pdus.len());
    while let Some(pdu) = pdus.next().await {
        formatted.extend(pdu.ok());
    }

    formatted
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

pub(super) async fn auth_chain_ids<'a, I>(
    services: &'a Services,
    room_id: &'a RoomId,
    starting_events: I,
) -> Result<Vec<OwnedEventId>>
where
    I: Iterator<Item = &'a EventId> + Clone + Debug + ExactSizeIterator + Send + 'a,
{
    let mut ids = Vec::new();
    let mut chain = pin!(
        services
            .rooms
            .auth_chain
            .event_ids_iter(room_id, starting_events)
    );
    while let Some(id) = chain.next().await {
        ids.push(id?);
    }

    Ok(ids)
}
