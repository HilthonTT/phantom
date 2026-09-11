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

#[implement(Service)]
async fn authorized_against(
    &self,
    room_version: &RoomVersion,
    incoming_pdu: &PduEvent,
    state: &StateAtEvent,
) -> Result<bool> {
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
