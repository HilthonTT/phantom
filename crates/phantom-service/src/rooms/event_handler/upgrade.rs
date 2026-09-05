//! Turning a believed event into part of the room.
//!
//! Everything up to here established that the event is genuine and worked out
//! what the room looked like when it happened. What is left is the decision
//! the room's rules make, and it has two halves that are easy to confuse.
//!
//! **Against the state at the event**, the rules say whether the event was
//! ever valid. An event that fails here never happened as far as this server
//! is concerned, and is rejected outright.
//!
//! **Against the room's current state**, the rules say whether it is valid
//! *now*. An event that passes the first check and fails this one is
//! soft-failed: the rest of the room accepted it, so this server records that
//! it exists and lets later events build on it, but does not show it to anyone
//! and does not let it change the room's state. The usual cause is an event
//! from someone who has since been banned, arriving late.
//!
//! Only after both is the room's state moved on, and only then is the event
//! appended — in that order, so that nothing can read the event out of a room
//! whose state does not yet account for it.

use std::{borrow::Borrow, collections::HashSet, iter::once, sync::Arc};

use futures::StreamExt;
use phantom_core::{
    Err, Result, debug, debug_info, err, implement,
    matrix::{
        pdu::{PduEvent, RawPduId},
        state_res::{self, RoomVersion},
    },
    trace,
};
use ruma::{CanonicalJsonObject, OwnedEventId, RoomId, ServerName, events::StateEventType};

use super::{Service, state::StateAtEvent};
use crate::rooms::state_compressor::CompressedState;

/// Places a validated outlier in the room's timeline.
///
/// `Some(id)` where the event was appended, `None` where it was soft-failed or
/// was already there.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(event_id = %incoming_pdu.event_id))]
pub(super) async fn upgrade_outlier_to_timeline_pdu(
    &self,
    incoming_pdu: PduEvent,
    value: CanonicalJsonObject,
    create_event: &PduEvent,
    origin: &ServerName,
    room_id: &RoomId,
) -> Result<Option<RawPduId>> {
    if self
        .services
        .timeline
        .pdu_exists(&incoming_pdu.event_id)
        .await
    {
        return Ok(None);
    }

    // An event already soft-failed once stays soft-failed. Re-deciding it
    // against a state that has moved on since would let a banned user's event
    // appear because the ban was later lifted.
    if self.is_soft_failed(&incoming_pdu.event_id).await {
        debug!("Event was soft failed before");
        return Ok(None);
    }

    let room_version_id = self.services.state.get_room_version(room_id).await?;
    let room_version = RoomVersion::new(&room_version_id)?;

    let state_at_event = self
        .state_at_event(
            &incoming_pdu,
            create_event,
            origin,
            room_id,
            &room_version_id,
        )
        .await?;

    let authorized = self
        .authorized_against(&room_version, &incoming_pdu, &state_at_event)
        .await?;

    if !authorized {
        return Err!(Request(Forbidden(
            "Event did not pass auth check against the state at the event."
        )));
    }

    let soft_fail = !self
        .authorized_against_current(&room_version, &incoming_pdu, room_id)
        .await?;

    // The state after the event is the state at it, plus the event itself
    // where the event is a state event.
    let state_after = self
        .state_after(&incoming_pdu, state_at_event.clone())
        .await;

    let compressed_state_at_event = self.compress(&state_at_event).await;

    let state_lock = self.services.state.mutex.lock(room_id).await;

    let extremities = self.extremities(room_id, &incoming_pdu).await;

    if soft_fail {
        debug_info!("Soft failing event {}", incoming_pdu.event_id);

        self.services
            .timeline
            .append_incoming_pdu(
                &incoming_pdu,
                value,
                extremities.iter().map(Borrow::borrow),
                compressed_state_at_event,
                true,
                &state_lock,
            )
            .await?;

        self.mark_soft_failed(&incoming_pdu.event_id);

        return Ok(None);
    }

    // A state event moves the room on; a message does not, and resolving state
    // for one would be work with a foregone conclusion.
    if incoming_pdu.state_key.is_some() {
        let resolved = self
            .resolve_state(room_id, &room_version_id, state_after)
            .await?;

        self.install_state(room_id, resolved, &state_lock).await?;
    }

    let pdu_id = self
        .services
        .timeline
        .append_incoming_pdu(
            &incoming_pdu,
            value,
            extremities.iter().map(Borrow::borrow),
            compressed_state_at_event,
            false,
            &state_lock,
        )
        .await?;

    drop(state_lock);

    Ok(pdu_id)
}

/// The state at the event, from our own record where possible and from the
/// sending server where not.
#[implement(Service)]
async fn state_at_event(
    &self,
    incoming_pdu: &PduEvent,
    create_event: &PduEvent,
    origin: &ServerName,
    room_id: &RoomId,
    room_version_id: &ruma::RoomVersionId,
) -> Result<StateAtEvent> {
    let local = if incoming_pdu.prev_events.len() == 1 {
        self.state_at_incoming_degree_one(incoming_pdu).await?
    } else {
        self.state_at_incoming_resolved(incoming_pdu, room_id, room_version_id)
            .await?
    };

    if let Some(state) = local {
        return Ok(state);
    }

    self.fetch_state(
        origin,
        create_event,
        room_id,
        room_version_id,
        &incoming_pdu.event_id,
    )
    .await?
    .ok_or_else(|| {
        err!(Request(NotFound(
            "The state at the event could not be established."
        )))
    })
}

/// Runs the room's rules over the event against a state given as short ids.
#[implement(Service)]
async fn authorized_against(
    &self,
    room_version: &RoomVersion,
    incoming_pdu: &PduEvent,
    state: &StateAtEvent,
) -> Result<bool> {
    // The closure owns what it is given rather than borrowing it: the state
    // resolution API hands it a reference per call, and a future holding that
    // reference cannot outlive the call that made it.
    let fetch = |event_type: &StateEventType, state_key: &str| {
        let event_type = event_type.clone();
        let state_key = state_key.to_owned();

        async move {
            let shortstatekey = self
                .services
                .short
                .get_shortstatekey(&event_type, &state_key)
                .await
                .ok()?;

            let event_id = state.get(&shortstatekey)?;

            self.services.timeline.get_pdu(event_id).await.ok()
        }
    };

    state_res::auth_check(room_version, incoming_pdu, None, fetch)
        .await
        .map_err(Into::into)
}

/// Runs the room's rules over the event against the room as it stands.
#[implement(Service)]
async fn authorized_against_current(
    &self,
    room_version: &RoomVersion,
    incoming_pdu: &PduEvent,
    room_id: &RoomId,
) -> Result<bool> {
    let fetch = |event_type: &StateEventType, state_key: &str| {
        let event_type = event_type.clone();
        let state_key = state_key.to_owned();

        async move {
            self.services
                .state_accessor
                .room_state_get(room_id, &event_type, &state_key)
                .await
                .ok()
        }
    };

    state_res::auth_check(room_version, incoming_pdu, None, fetch)
        .await
        .map_err(Into::into)
}

/// The state at the event with the event itself applied.
#[implement(Service)]
async fn state_after(&self, incoming_pdu: &PduEvent, mut state: StateAtEvent) -> StateAtEvent {
    if let Some(state_key) = &incoming_pdu.state_key {
        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&incoming_pdu.kind.to_string().into(), state_key)
            .await;

        state.insert(shortstatekey, incoming_pdu.event_id.clone());
    }

    state
}

/// The room's forward extremities once this event is in it.
///
/// The event's own predecessors stop being extremities — the event is now what
/// comes after them — and the event itself becomes one. Anything else that was
/// an extremity stays one, unless this server never had it, which happens when
/// an extremity was recorded from a state fetch rather than from an event.
#[implement(Service)]
async fn extremities(&self, room_id: &RoomId, incoming_pdu: &PduEvent) -> Vec<OwnedEventId> {
    let superseded: HashSet<&OwnedEventId> = incoming_pdu.prev_events.iter().collect();

    let mut extremities: Vec<OwnedEventId> = self
        .services
        .state
        .get_forward_extremities(room_id)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter(|event_id| !superseded.contains(event_id))
        .collect();

    let mut kept = Vec::with_capacity(extremities.len().saturating_add(1));

    for event_id in extremities.drain(..) {
        if self.services.timeline.pdu_exists(&event_id).await {
            kept.push(event_id);
        } else {
            trace!("Dropping extremity {event_id}, which this server does not hold");
        }
    }

    kept.extend(once(incoming_pdu.event_id.clone()));
    kept
}

/// Compresses a state so it can be stored against the event.
#[implement(Service)]
async fn compress(&self, state: &StateAtEvent) -> Arc<CompressedState> {
    let compressed: CompressedState = self
        .services
        .state_compressor
        .compress_state_events(state.iter().map(|(key, id)| (key, id.borrow())))
        .collect()
        .await;

    Arc::new(compressed)
}

/// Makes a resolved state the room's, where it differs from what the room has.
#[implement(Service)]
async fn install_state(
    &self,
    room_id: &RoomId,
    resolved: Arc<CompressedState>,
    state_lock: &crate::rooms::state::RoomMutexGuard,
) -> Result {
    let saved = self
        .services
        .state_compressor
        .save_state(room_id, resolved)
        .await?;

    self.services
        .state
        .force_state(
            room_id,
            saved.shortstatehash,
            saved.added,
            saved.removed,
            state_lock,
        )
        .await
}
