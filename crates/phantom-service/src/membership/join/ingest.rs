//! Ingesting a `send_join` response: outliers, the auth check, and forced state.

use std::{borrow::Borrow, sync::Arc};

use futures::StreamExt;
use phantom_core::{
    Err, Result, debug, debug_error, debug_warn, err, implement, info,
    matrix::{
        PduEvent,
        state_res::{self, RoomVersion},
    },
};
use ruma::{CanonicalJsonValue, RoomId, RoomVersionId, events::StateEventType};
use serde_json::value::RawValue as RawJsonValue;

use super::{StateIds, remote::parse_federation_pdu};
use crate::{
    membership::Service,
    rooms::{
        state::RoomMutexGuard,
        state_compressor::{CompressedState, HashSetCompressStateEvent},
    },
};

#[implement(Service)]
pub(super) async fn ingest_send_join_state(
    &self,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    state_pdus: &[Box<RawJsonValue>],
) -> StateIds {
    info!(
        events = state_pdus.len(),
        "Going through send_join response room_state..."
    );

    let cork = self.services.db.engine.cork_and_flush();

    let mut state = StateIds::new();

    for pdu in state_pdus {
        let (event_id, value) = match self
            .services
            .server_keys
            .validate_and_add_event_id_no_fetch(pdu, room_version_id)
            .await
        {
            Ok(validated) => validated,
            Err(e) => {
                debug_error!("Invalid send_join state event: {e:?}");
                continue;
            }
        };

        let parsed = match parse_federation_pdu(room_id, &event_id, value.clone()) {
            Ok(parsed) => parsed,
            Err(e) => {
                debug_warn!("Invalid PDU {event_id:?} in send_join response: {e:?}");
                continue;
            }
        };

        self.services.outlier.add_pdu_outlier(&event_id, &value);

        if let Some(state_key) = &parsed.state_key {
            let shortstatekey = self
                .services
                .short
                .get_or_create_shortstatekey(&parsed.kind.to_string().into(), state_key)
                .await;

            state.insert(shortstatekey, parsed.event_id);
        }
    }

    drop(cork);

    state
}

#[implement(Service)]
pub(super) async fn ingest_send_join_auth_chain(
    &self,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    auth_chain: &[Box<RawJsonValue>],
) {
    info!(
        events = auth_chain.len(),
        "Going through send_join response auth_chain..."
    );

    let requires_room_id = room_version_id
        .rules()
        .is_none_or(|rules| rules.event_format.require_room_create_room_id);

    let cork = self.services.db.engine.cork_and_flush();

    for pdu in auth_chain {
        let (event_id, mut value) = match self
            .services
            .server_keys
            .validate_and_add_event_id_no_fetch(pdu, room_version_id)
            .await
        {
            Ok(validated) => validated,
            Err(e) => {
                debug_error!("Invalid send_join auth_chain event: {e:?}");
                continue;
            }
        };

        let is_create =
            value.get("type").and_then(CanonicalJsonValue::as_str) == Some("m.room.create");

        if !requires_room_id && is_create {
            value.insert(
                "room_id".into(),
                CanonicalJsonValue::String(room_id.as_str().into()),
            );
        }

        self.services.outlier.add_pdu_outlier(&event_id, &value);
    }

    drop(cork);
}

#[implement(Service)]
pub(super) async fn auth_check_join(
    &self,
    room_version_id: &RoomVersionId,
    join_pdu: &PduEvent,
    state: &StateIds,
) -> Result {
    let room_version = RoomVersion::new(room_version_id)?;

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

    let authorized = state_res::auth_check(&room_version, join_pdu, None, fetch)
        .await
        .map_err(|e| {
            err!(Request(Forbidden(error!(
                "send_join auth check failed: {e:?}"
            ))))
        })?;

    if !authorized {
        return Err!(Request(Forbidden(
            "Join event failed the send_join auth check."
        )));
    }

    Ok(())
}

#[implement(Service)]
pub(in crate::membership) async fn apply_state(
    &self,
    room_id: &RoomId,
    state: &StateIds,
    state_lock: &RoomMutexGuard,
) -> Result {
    info!(events = state.len(), "Compressing state...");

    let compressed: CompressedState = self
        .services
        .state_compressor
        .compress_state_events(
            state
                .iter()
                .map(|(shortstatekey, event_id)| (shortstatekey, event_id.borrow())),
        )
        .collect()
        .await;

    debug!("Saving compressed state...");

    let HashSetCompressStateEvent {
        shortstatehash,
        added,
        removed,
    } = self
        .services
        .state_compressor
        .save_state(room_id, Arc::new(compressed))
        .await?;

    debug!(state_hash = ?shortstatehash, "Forcing state for new room...");

    self.services
        .state
        .force_state(room_id, shortstatehash, added, removed, state_lock)
        .await
}
