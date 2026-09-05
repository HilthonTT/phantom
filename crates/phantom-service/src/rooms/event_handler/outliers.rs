//! Believing an event without yet placing it.
//!
//! An outlier is an event this server has checked and stored but has not put
//! in a room's timeline, because it does not know where in the timeline it
//! goes. Almost every event arrives this way first: an event is authorized by
//! naming the state events that permit it, those events have to be in hand
//! before the check can run, and fetching them pulls in their own auth events
//! in turn.
//!
//! Two checks make an outlier. Its **signatures** must verify against the keys
//! of the servers that signed it, and its own recomputed id must match — an
//! event whose content hash does not match is accepted in redacted form, since
//! that is what a redaction looks like from outside. And it must **pass the
//! room's rules against its own `auth_events`**, which is a weaker statement
//! than passing against the room's state and is all that can be said about an
//! event whose place is not yet known.

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

/// Validates one event and stores it as an outlier.
///
/// `auth_events_known` says the event's `auth_events` are already in this
/// server's store, which is the case when a batch of events has just been
/// fetched together. Where it is false, the auth events are fetched here.
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
    // An outlier already validated once needs no second opinion: nothing about
    // an event changes, and the check is expensive enough to be worth not
    // repeating for every event that names it.
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

/// Verifies an event's signatures, redacting it where its content hash does
/// not match.
///
/// A content hash that does not match is not evidence of tampering as long as
/// the signatures hold: it is what a redacted event looks like, since a
/// redaction removes content the signature was taken over. So the event is
/// accepted in the form the signature does cover — its redacted form — rather
/// than rejected.
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

    // The id is not part of an event over federation, and everything below
    // reads it off the object rather than passing it alongside.
    value.insert(
        "event_id".to_owned(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok(value)
}

/// The event's `auth_events`, as a state map to check it against.
///
/// Two things are refused rather than merely ignored, because both mean the
/// event is lying about what authorizes it: naming the same state key twice,
/// and naming an event from a different room.
#[implement(Service)]
async fn auth_state_of(
    &self,
    incoming_pdu: &PduEvent,
    create_event: &PduEvent,
) -> Result<HashMap<state_res::TypeStateKey, PduEvent>> {
    let mut auth_state = HashMap::with_capacity(incoming_pdu.auth_events.len());

    for auth_event_id in &incoming_pdu.auth_events {
        let Ok(auth_event) = self.services.timeline.get_pdu(auth_event_id).await else {
            // An auth event we could not obtain leaves the check unable to
            // find the state it needs, which fails it below rather than here:
            // the missing event may be one the rules do not consult.
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

    // The create event authorizes everything in the room and is not always
    // named explicitly by the events that depend on it.
    auth_state
        .entry((StateEventType::RoomCreate, String::new().into()))
        .or_insert_with(|| create_event.clone());

    Ok(auth_state)
}

/// Obtains events, and everything needed to authorize them, from `origin`.
///
/// The auth graph is walked depth-first so that an event is handled only after
/// everything authorizing it has been, which is what lets each one be checked
/// with its auth events already in the store. An event that cannot be fetched
/// or will not validate is dropped from the walk and remembered as bad; its
/// dependents will fail their own auth check for the lack of it, which is the
/// correct outcome and not one worth a separate error path.
///
/// Returns what could be obtained, in the order asked for. A caller getting
/// back fewer events than it asked for is the ordinary case, not a failure.
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
        // Boxed to break the type cycle: handling an event may fetch the
        // events that authorize it, which handles those in turn.
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

/// Fetches `events` and their transitive `auth_events`, in an order that puts
/// every event after the ones that authorize it.
///
/// Depth-first with an explicit stack rather than recursion: the auth graph of
/// a busy room is thousands of events deep in the worst case, which is a stack
/// overflow rather than a slow request.
#[implement(Service)]
async fn fetch_auth_graph(
    &self,
    origin: &ServerName,
    events: &[OwnedEventId],
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Vec<(OwnedEventId, CanonicalJsonObject)> {
    /// A frame is visited twice: once to fetch it and push its dependencies,
    /// and again once those have been dealt with.
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

        // Already held, in the timeline or as an outlier: nothing to fetch,
        // and nothing below it to walk either, since it was validated when it
        // was stored.
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

/// Asks `origin` for one event, and checks that what comes back is the event
/// that was asked for.
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

    // The id is the hash of the event, so a mismatch means the server sent a
    // different event than the one asked for — not an error to recover from by
    // using what arrived.
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

/// The `auth_events` of an event, read out of its JSON.
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
