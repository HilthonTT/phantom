use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use futures::StreamExt;
use phantom_core::{debug, debug_warn, implement, matrix::pdu::PduEvent, trace};
use ruma::{
    CanonicalJsonObject, MilliSecondsSinceUnixEpoch, OwnedEventId, RoomId, RoomVersionId,
    ServerName, UInt, api::federation::event::get_missing_events,
};

use super::Service;

const MISSING_EVENTS_LIMIT: u16 = 20;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub(super) async fn fill_gap(
    &self,
    origin: &ServerName,
    create_event: &PduEvent,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    incoming_pdu: &PduEvent,
    first_ts_in_room: UInt,
) {
    let started = Instant::now();

    let missing: Vec<OwnedEventId> = {
        let mut missing = Vec::new();

        for prev_event_id in &incoming_pdu.prev_events {
            if !self.services.timeline.pdu_exists(prev_event_id).await {
                missing.push(prev_event_id.clone());
            }
        }

        missing
    };

    if missing.is_empty() {
        trace!("No gap before {}", incoming_pdu.event_id);
        return;
    }

    debug!(
        missing = missing.len(),
        "Filling the gap before {}", incoming_pdu.event_id
    );

    let events = self
        .fetch_missing_events(origin, room_id, room_version_id, incoming_pdu, &missing)
        .await;

    for (event_id, pdu) in sorted_oldest_first(events) {
        if started.elapsed() > self.prev_event_budget {
            debug_warn!(
                "Gave up filling the gap before {} after {:?}",
                incoming_pdu.event_id,
                started.elapsed()
            );
            return;
        }

        if pdu.origin_server_ts < first_ts_in_room {
            trace!("Skipping {event_id}, which predates the room's history here");
            continue;
        }

        if self.services.timeline.pdu_exists(&event_id).await {
            continue;
        }

        let value = match self.services.outlier.get_outlier_pdu_json(&event_id).await {
            Ok(value) => value,
            Err(e) => {
                debug_warn!("Gap event {event_id} was not stored as an outlier: {e}");
                continue;
            }
        };

        if let Err(e) = self
            .upgrade_outlier_to_timeline_pdu(pdu, value, create_event, origin, room_id)
            .await
        {
            debug_warn!("Gap event {event_id} could not be placed: {e}");
        }
    }
}

#[implement(Service)]
async fn fetch_missing_events(
    &self,
    origin: &ServerName,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    incoming_pdu: &PduEvent,
    missing: &[OwnedEventId],
) -> HashMap<OwnedEventId, PduEvent> {
    let earliest: Vec<OwnedEventId> = self
        .services
        .state
        .get_forward_extremities(room_id)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
        .await;

    let mut request = get_missing_events::v1::Request::new(
        room_id.to_owned(),
        earliest,
        vec![incoming_pdu.event_id.clone()],
    );

    request.limit = UInt::from(MISSING_EVENTS_LIMIT);
    request.min_depth = UInt::MIN;

    let mut wanted: HashSet<OwnedEventId> = missing.iter().cloned().collect();
    let mut found = HashMap::new();

    match self.services.federation.execute(origin, request).await {
        Ok(response) => {
            for pdu in response.events {
                let Ok((event_id, value)) =
                    phantom_core::matrix::pdu::gen_event_id_canonical_json(&pdu, room_version_id)
                else {
                    continue;
                };

                if let Some(pdu) = self
                    .accept_gap_event(origin, room_id, &event_id, value)
                    .await
                {
                    wanted.remove(&event_id);
                    found.insert(event_id, pdu);
                }
            }
        }
        Err(e) => debug_warn!("{origin} would not fill the gap: {e}"),
    }

    for event_id in wanted {
        if let Ok(pdu) = self.services.timeline.get_pdu(&event_id).await {
            found.insert(event_id, pdu);
        }
    }

    found
}

#[implement(Service)]
async fn accept_gap_event(
    &self,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &ruma::EventId,
    value: CanonicalJsonObject,
) -> Option<PduEvent> {
    let create_event = self
        .services
        .state_accessor
        .room_state_get(room_id, &ruma::events::StateEventType::RoomCreate, "")
        .await
        .ok()?;

    match self
        .handle_outlier_pdu(origin, &create_event, event_id, room_id, value, false)
        .await
    {
        Ok((pdu, _)) => Some(pdu),
        Err(e) => {
            debug_warn!("Gap event {event_id} would not validate: {e}");
            self.mark_bad(event_id);

            None
        }
    }
}

fn sorted_oldest_first(events: HashMap<OwnedEventId, PduEvent>) -> Vec<(OwnedEventId, PduEvent)> {
    let mut events: Vec<_> = events.into_iter().collect();

    events.sort_by(|(a_id, a), (b_id, b)| {
        MilliSecondsSinceUnixEpoch(a.origin_server_ts)
            .cmp(&MilliSecondsSinceUnixEpoch(b.origin_server_ts))
            .then_with(|| a_id.cmp(b_id))
    });

    events
}
