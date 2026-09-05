//! What the room's state was when an event happened, and what it is now that
//! the event has.
//!
//! An event is authorized against the state of the room *at that event*, which
//! is not the state now and not necessarily a state this server ever held.
//! There are three ways to arrive at it, tried in this order because that is
//! their order of cost and of trustworthiness:
//!
//! 1. The event has one predecessor and we have it. Then the state at the
//!    event is the state after that predecessor, which is a lookup.
//! 2. The event has several predecessors and we have them all. Their states
//!    may disagree — that is what several predecessors means — so they are
//!    resolved against each other. Still no network.
//! 3. We do not have the predecessors. Then the only source is the server that
//!    sent us the event, which is asked for the state at it; every event in
//!    the answer is fetched and validated before any of it is believed.
//!
//! Afterwards the room's own state has to move on. [`resolve_state`] takes the
//! state the room was in and the state the new event implies, and resolves the
//! two, which is what stops a fork in the room from becoming a fork in this
//! server's idea of it.
//!
//! [`resolve_state`]: Service::resolve_state

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::StreamExt;
use phantom_core::{
    Result, debug, debug_warn, err, implement,
    matrix::{
        pdu::PduEvent,
        state_res::{self, StateMap},
    },
    stream::{IterStream, ReadyExt, automatic_width},
    trace, warn,
};
use ruma::{
    OwnedEventId, RoomId, RoomVersionId, ServerName, api::federation::event::get_room_state_ids,
    events::StateEventType,
};

use super::Service;
use crate::rooms::{
    short::{ShortEventId, ShortStateKey},
    state_compressor::CompressedState,
};

/// The room's state at an event, keyed the way the compressor wants it.
pub(super) type StateAtEvent = HashMap<ShortStateKey, OwnedEventId>;

/// The state at an event whose single predecessor this server holds.
///
/// `None` where the predecessor is not held, or is held without a record of
/// the state at it — an outlier, most often.
#[implement(Service)]
pub(super) async fn state_at_incoming_degree_one(
    &self,
    incoming_pdu: &PduEvent,
) -> Result<Option<StateAtEvent>> {
    let prev_event = &incoming_pdu.prev_events[0];

    let Ok(prev_event_sstatehash) = self
        .services
        .state_accessor
        .pdu_shortstatehash(prev_event)
        .await
    else {
        return Ok(None);
    };

    let mut state: StateAtEvent = self
        .services
        .state_accessor
        .state_full_ids(prev_event_sstatehash)
        .collect()
        .await;

    debug!("Using the state after the only previous event");

    let Ok(prev_pdu) = self.services.timeline.get_pdu(prev_event).await else {
        return Ok(None);
    };

    // The state *after* the predecessor includes the predecessor itself, where
    // it was a state event. The stored hash is the state it was authorized
    // against, which is the state before it.
    if let Some(state_key) = &prev_pdu.state_key {
        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&prev_pdu.kind.to_string().into(), state_key)
            .await;

        state.insert(shortstatekey, prev_event.clone());
    }

    Ok(Some(state))
}

/// The state at an event with several predecessors, by resolving theirs.
///
/// `None` where any predecessor is missing: resolving a subset of the forks
/// would produce a state that looks authoritative and is not.
#[implement(Service)]
pub(super) async fn state_at_incoming_resolved(
    &self,
    incoming_pdu: &PduEvent,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
) -> Result<Option<StateAtEvent>> {
    debug!(
        prev_events = incoming_pdu.prev_events.len(),
        "Resolving the state after several previous events"
    );

    let mut fork_states = Vec::with_capacity(incoming_pdu.prev_events.len());
    let mut auth_chain_sets = Vec::with_capacity(incoming_pdu.prev_events.len());

    for prev_event_id in &incoming_pdu.prev_events {
        let Ok(prev_event) = self.services.timeline.get_pdu(prev_event_id).await else {
            debug!("Missing previous event {prev_event_id}");
            return Ok(None);
        };

        let Ok(sstatehash) = self
            .services
            .state_accessor
            .pdu_shortstatehash(prev_event_id)
            .await
        else {
            debug!("No state recorded at previous event {prev_event_id}");
            return Ok(None);
        };

        let mut leaf_state: StateAtEvent = self
            .services
            .state_accessor
            .state_full_ids(sstatehash)
            .collect()
            .await;

        if let Some(state_key) = &prev_event.state_key {
            let shortstatekey = self
                .services
                .short
                .get_or_create_shortstatekey(&prev_event.kind.to_string().into(), state_key)
                .await;

            leaf_state.insert(shortstatekey, prev_event_id.clone());
        }

        let auth_chain: HashSet<OwnedEventId> = self
            .auth_chain_of(room_id, leaf_state.values())
            .await
            .into_iter()
            .collect();

        auth_chain_sets.push(auth_chain);
        fork_states.push(self.spell_out(leaf_state).await);
    }

    let resolved = state_res::resolve(
        room_version_id,
        &fork_states,
        &auth_chain_sets,
        &|event_id| self.fetch_for_resolution(event_id),
        &|event_id| self.exists_for_resolution(event_id),
        automatic_width(),
    )
    .await
    .map_err(|e| err!(Database(warn!("State resolution failed: {e}"))))?;

    Ok(Some(self.shorten(resolved).await))
}

/// The state at an event as the server that sent it says it was.
///
/// Every event named in the answer is fetched and validated as an outlier
/// before any of it is used: the sending server is being asked what the state
/// was, not being trusted about it. State ids are asked for rather than the
/// state itself for the same reason — the events have to be fetched and
/// checked either way, and asking for ids does not invite a server to hand us
/// a megabyte of state it invented.
#[implement(Service)]
pub(super) async fn fetch_state(
    &self,
    origin: &ServerName,
    create_event: &PduEvent,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    event_id: &ruma::EventId,
) -> Result<Option<StateAtEvent>> {
    debug!("Asking {origin} for the state at {event_id}");

    let request = get_room_state_ids::v1::Request::new(event_id.to_owned(), room_id.to_owned());

    let response = match self.services.federation.execute(origin, request).await {
        Ok(response) => response,
        Err(e) => {
            debug_warn!("{origin} would not give the state at {event_id}: {e}");
            return Ok(None);
        }
    };

    self.fetch_and_handle_outliers(
        origin,
        &response.auth_chain_ids,
        create_event,
        room_id,
        room_version_id,
    )
    .await;

    let events = self
        .fetch_and_handle_outliers(
            origin,
            &response.pdu_ids,
            create_event,
            room_id,
            room_version_id,
        )
        .await;

    let mut state = StateAtEvent::with_capacity(events.len());

    for pdu in events {
        let Some(state_key) = &pdu.state_key else {
            trace!(
                "Ignoring non-state event {} in a state response",
                pdu.event_id
            );
            continue;
        };

        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&pdu.kind.to_string().into(), state_key)
            .await;

        // Two events at one state key is a state the room could never have
        // been in, so the answer is not usable as a whole.
        if state
            .insert(shortstatekey, pdu.event_id.clone())
            .is_some_and(|previous| previous != pdu.event_id)
        {
            warn!("{origin} sent two events for one state key at {event_id}");
            return Ok(None);
        }
    }

    // A room's state always contains its create event. An answer without one
    // is not a state, whatever else is in it.
    let create_key = self
        .services
        .short
        .get_or_create_shortstatekey(&StateEventType::RoomCreate, "")
        .await;

    if !state.contains_key(&create_key) {
        warn!("{origin} sent a state at {event_id} with no create event");
        return Ok(None);
    }

    Ok(Some(state))
}

/// Resolves the room's current state against the state a new event implies,
/// and stores the result.
///
/// Returns the resolved state compressed, ready to become the room's.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub(super) async fn resolve_state(
    &self,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    incoming_state: StateAtEvent,
) -> Result<Arc<CompressedState>> {
    debug!("Resolving the room's state against the incoming event's");

    let current_sstatehash = self
        .services
        .state
        .get_room_shortstatehash(room_id)
        .await
        .map_err(|e| err!(Database("No state for {room_id}: {e}")))?;

    let current_state: StateAtEvent = self
        .services
        .state_accessor
        .state_full_ids(current_sstatehash)
        .collect()
        .await;

    let mut auth_chain_sets = Vec::with_capacity(2);
    let mut fork_states = Vec::with_capacity(2);

    for state in [current_state, incoming_state] {
        let auth_chain: HashSet<OwnedEventId> = self
            .auth_chain_of(room_id, state.values())
            .await
            .into_iter()
            .collect();

        auth_chain_sets.push(auth_chain);
        fork_states.push(self.spell_out(state).await);
    }

    let resolved = state_res::resolve(
        room_version_id,
        &fork_states,
        &auth_chain_sets,
        &|event_id| self.fetch_for_resolution(event_id),
        &|event_id| self.exists_for_resolution(event_id),
        automatic_width(),
    )
    .await
    .map_err(|e| err!(Database(warn!("State resolution failed: {e}"))))?;

    let resolved = self.shorten(resolved).await;

    let compressed: CompressedState = self
        .services
        .state_compressor
        .compress_state_events(resolved.iter().map(|(key, id)| (key, id.borrow())))
        .collect()
        .await;

    Ok(Arc::new(compressed))
}

/// The auth chain of a set of events, as event ids.
#[implement(Service)]
async fn auth_chain_of<'a, I>(&self, room_id: &RoomId, starting_events: I) -> Vec<OwnedEventId>
where
    I: Iterator<Item = &'a OwnedEventId> + Send,
{
    let starting_events: Vec<&ruma::EventId> =
        starting_events.map(|event_id| event_id.borrow()).collect();

    let Ok(chain) = self
        .services
        .auth_chain
        .get_auth_chain(room_id, starting_events.into_iter())
        .await
    else {
        return Vec::new();
    };

    self.spell_out_short(chain).await
}

/// Short event ids spelled back out.
#[implement(Service)]
async fn spell_out_short(&self, short_ids: Vec<ShortEventId>) -> Vec<OwnedEventId> {
    self.services
        .short
        .multi_get_eventid_from_short(short_ids.into_iter().stream())
        .ready_filter_map(Result::ok)
        .collect()
        .await
}

/// A state keyed by short state key, keyed by type and state key instead.
///
/// State resolution works in the spec's terms rather than this server's short
/// ids, since what it compares is what the room's rules are written about.
#[implement(Service)]
async fn spell_out(&self, state: StateAtEvent) -> StateMap<OwnedEventId> {
    let mut spelled = StateMap::with_capacity(state.len());

    for (shortstatekey, event_id) in state {
        let Ok((event_type, state_key)) = self
            .services
            .short
            .get_statekey_from_short(shortstatekey)
            .await
        else {
            continue;
        };

        spelled.insert((event_type, state_key), event_id);
    }

    spelled
}

/// The inverse of [`spell_out`].
///
/// [`spell_out`]: Service::spell_out
#[implement(Service)]
async fn shorten(&self, state: StateMap<OwnedEventId>) -> StateAtEvent {
    let mut shortened = StateAtEvent::with_capacity(state.len());

    for ((event_type, state_key), event_id) in state {
        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&event_type, &state_key)
            .await;

        shortened.insert(shortstatekey, event_id);
    }

    shortened
}

/// The event fetcher state resolution is given.
#[implement(Service)]
async fn fetch_for_resolution(&self, event_id: OwnedEventId) -> Option<PduEvent> {
    self.services.timeline.get_pdu(&event_id).await.ok()
}

/// The existence check state resolution is given.
///
/// Separate from the fetcher because resolution asks about far more events
/// than it reads, and answering with a key lookup rather than a value read is
/// most of what makes the conflicted set affordable.
#[implement(Service)]
async fn exists_for_resolution(&self, event_id: OwnedEventId) -> bool {
    self.services.timeline.pdu_exists(&event_id).await
}
