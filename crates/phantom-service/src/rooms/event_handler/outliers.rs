use std::collections::{HashMap, HashSet};

use futures::future::ready;
use phantom_core::{
    Err, Result, debug, debug_warn, err, implement,
    matrix::{
        Event,
        pdu::PduEvent,
        state_res::{self, RoomVersion},
    },
    trace, warn,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedEventId, RoomId, RoomVersionId,
    ServerName, api::federation::event::get_event, events::StateEventType, signatures::Verified,
};

use super::Service;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(%event_id))]
pub(super) async fn handle_outlier_pdu(
    &self,
    origin: &ServerName,
    create_event: &PduEvent,
    event_id: &EventId,
    room_id: &RoomId,
    value: CanonicalJsonObject,
    auth_events_known: bool,
) -> Result<(PduEvent, CanonicalJsonObject)> {
    if let Ok(pdu) = self.services.outlier.get_pdu_outlier(event_id).await
        && let Ok(json) = self.services.outlier.get_outlier_pdu_json(event_id).await
    {
        return Ok((pdu, json));
    }

    let room_version_id = self.services.state.get_room_version(room_id).await?;
    let room_version = RoomVersion::new(&room_version_id)?;

    let value = self.verified(event_id, value, &room_version_id).await?;

    let incoming_pdu = PduEvent::from_id_val(event_id, value.clone())
        .map_err(|e| err!(Request(InvalidParam("Event is not a valid PDU: {e}"))))?;

    if incoming_pdu.room_id() != room_id {
        return Err!(Request(InvalidParam(
            "Event is for a different room than the one it arrived under."
        )));
    }

    if !auth_events_known {
        self.fetch_and_handle_outliers(
            origin,
            &incoming_pdu.auth_events,
            create_event,
            room_id,
            &room_version_id,
        )
        .await;
    }

    let auth_state = self.auth_state_of(&incoming_pdu, create_event).await?;

    let authorized = state_res::auth_check(&room_version, &incoming_pdu, None, |ty, key| {
        ready(auth_state.get(&(ty.clone(), key.into())).cloned())
    })
    .await?;

    if !authorized {
        return Err!(Request(Forbidden(
            "Event did not pass auth check against its own auth events."
        )));
    }

    trace!("Storing {event_id} as an outlier");
    self.services.outlier.add_pdu_outlier(event_id, &value);

    Ok((incoming_pdu, value))
}

#[implement(Service)]
async fn verified(
    &self,
    event_id: &EventId,
    value: CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> Result<CanonicalJsonObject> {
    let rules = room_version_id
        .rules()
        .ok_or_else(|| err!(Request(UnsupportedRoomVersion("{room_version_id}"))))?;

    let verified = self
        .services
        .server_keys
        .verify_event(&value, Some(room_version_id))
        .await;

    let mut value = match verified {
        Ok(Verified::All) => value,
        Ok(Verified::Signatures) => {
            debug_warn!("Event {event_id} contents were redacted before it reached us");

            ruma::canonical_json::redact(value, &rules.redaction, None)
                .map_err(|e| err!(Request(InvalidParam("Event could not be redacted: {e}"))))?
        }
        Err(e) => {
            self.mark_bad(event_id);

            return Err!(Request(InvalidParam(debug_warn!(
                "Event {event_id} failed signature verification: {e}"
            ))));
        }
    };

    value.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok(value)
}

#[implement(Service)]
async fn auth_state_of(
    &self,
    incoming_pdu: &PduEvent,
    create_event: &PduEvent,
) -> Result<HashMap<state_res::TypeStateKey, PduEvent>> {
    let mut auth_state = HashMap::with_capacity(incoming_pdu.auth_events.len());

    for auth_event_id in &incoming_pdu.auth_events {
        let Ok(auth_event) = self.services.timeline.get_pdu(auth_event_id).await else {
            debug!("Missing auth event {auth_event_id}");
            continue;
        };

        if auth_event.room_id() != incoming_pdu.room_id() {
            return Err!(Request(InvalidParam(
                "Auth event {auth_event_id} is from a different room."
            )));
        }

        let Some(state_key) = auth_event.state_key.clone() else {
            return Err!(Request(InvalidParam(
                "Auth event {auth_event_id} is not a state event."
            )));
        };

        let key = (auth_event.kind.to_string().into(), state_key);

        if auth_state.insert(key, auth_event).is_some() {
            return Err!(Request(InvalidParam(
                "Event names two auth events for the same state key."
            )));
        }
    }

    auth_state
        .entry((StateEventType::RoomCreate, String::new().into()))
        .or_insert_with(|| create_event.clone());

    Ok(auth_state)
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(events = events.len()))]
pub(super) async fn fetch_and_handle_outliers(
    &self,
    origin: &ServerName,
    events: &[OwnedEventId],
    create_event: &PduEvent,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Vec<PduEvent> {
    let fetched = self
        .fetch_auth_graph(origin, events, room_id, room_version_id)
        .await;

    for (event_id, value) in fetched {
        let handled = Box::pin(self.handle_outlier_pdu(
            origin,
            create_event,
            &event_id,
            room_id,
            value,
            true,
        ))
        .await;

        match handled {
            Ok(_) => self.mark_good(&event_id),
            Err(e) => {
                debug_warn!("Fetched event {event_id} could not be handled: {e}");
                self.mark_bad(&event_id);
            }
        }
    }

    let mut found = Vec::with_capacity(events.len());

    for event_id in events {
        if let Ok(pdu) = self.services.timeline.get_pdu(event_id).await {
            found.push(pdu);
        }
    }

    found
}

#[implement(Service)]
async fn fetch_auth_graph(
    &self,
    origin: &ServerName,
    events: &[OwnedEventId],
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Vec<(OwnedEventId, CanonicalJsonObject)> {
    enum Step {
        Fetch(OwnedEventId),
        Emit(OwnedEventId),
    }

    let mut order = Vec::new();
    let mut fetched: HashMap<OwnedEventId, CanonicalJsonObject> = HashMap::new();
    let mut seen: HashSet<OwnedEventId> = HashSet::new();
    let mut stack: Vec<Step> = events.iter().cloned().map(Step::Fetch).rev().collect();

    while let Some(step) = stack.pop() {
        let event_id = match step {
            Step::Emit(event_id) => {
                if let Some(value) = fetched.remove(&event_id) {
                    order.push((event_id, value));
                }

                continue;
            }
            Step::Fetch(event_id) => event_id,
        };

        if !seen.insert(event_id.clone()) {
            continue;
        }

        if self.services.timeline.pdu_exists(&event_id).await {
            continue;
        }

        if self.is_backed_off(&event_id) {
            debug!("Not re-fetching {event_id} yet");
            continue;
        }

        let Some(value) = self
            .fetch_event(origin, &event_id, room_id, room_version_id)
            .await
        else {
            self.mark_bad(&event_id);
            continue;
        };

        let auth_events = auth_event_ids(&value);

        fetched.insert(event_id.clone(), value);
        stack.push(Step::Emit(event_id));

        for auth_event_id in auth_events.into_iter().rev() {
            stack.push(Step::Fetch(auth_event_id));
        }
    }

    order
}

#[implement(Service)]
async fn fetch_event(
    &self,
    origin: &ServerName,
    event_id: &EventId,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Option<CanonicalJsonObject> {
    let request = get_event::v1::Request::new(event_id.to_owned());

    let response = match self.services.federation.execute(origin, request).await {
        Ok(response) => response,
        Err(e) => {
            debug_warn!("Failed to fetch {event_id} from {origin}: {e}");
            return None;
        }
    };

    let (fetched_id, value) = match phantom_core::matrix::pdu::gen_event_id_canonical_json(
        &response.pdu,
        room_version_id,
    ) {
        Ok(fetched) => fetched,
        Err(e) => {
            debug_warn!("{origin} sent an unreadable event for {event_id}: {e}");
            return None;
        }
    };

    if fetched_id != event_id {
        warn!("{origin} sent {fetched_id} when asked for {event_id}");
        return None;
    }

    let claimed_room = value
        .get("room_id")
        .and_then(CanonicalJsonValue::as_str)
        .unwrap_or_default();

    if claimed_room != room_id.as_str() {
        warn!("{origin} sent {event_id} claiming to be in {claimed_room}, not {room_id}");
        return None;
    }

    Some(value)
}

fn auth_event_ids(value: &CanonicalJsonObject) -> Vec<OwnedEventId> {
    value
        .get("auth_events")
        .and_then(|auth_events| match auth_events {
            CanonicalJsonValue::Array(array) => Some(array),
            _ => None,
        })
        .into_iter()
        .flatten()
        .filter_map(CanonicalJsonValue::as_str)
        .filter_map(|id| EventId::parse(id).ok())
        .collect()
}
