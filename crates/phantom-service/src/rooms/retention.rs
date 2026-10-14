use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt, TryStreamExt};
use phantom_core::{
    Result, debug_info, debug_warn, expected, implement, info, matrix::pdu::PduEvent,
    runtime::server::Server, stream::TryReadyExt, time::now_millis, time::now_secs,
};
use phantom_database::{Deserialized, Json, Map, table};
use ruma::{CanonicalJsonObject, EventId, OwnedRoomId, RoomId, events::StateEventType};
use serde::Deserialize;

use crate::{
    Dep, rooms,
    rooms::{state::RoomMutexGuard, timeline::HistoryCutoff},
};

const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub struct Service {
    db: Data,
    services: Services,
}

struct Data {
    eventid_originalpdu: Arc<Map>,
    timeredacted_eventid: Arc<Map>,
}

struct Services {
    server: Arc<Server>,
    metadata: Dep<rooms::metadata::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

/// The part of `m.room.retention` enforced: how long a message may live, in
/// milliseconds. ruma has no type for the event, which never left its MSC.
#[derive(Deserialize)]
struct RetentionPolicy {
    max_lifetime: Option<u64>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                eventid_originalpdu: args.db[table::EVENTID_ORIGINALPDU].clone(),
                timeredacted_eventid: args.db[table::TIMEREDACTED_EVENTID].clone(),
            },
            services: Services {
                server: args.server.clone(),
                metadata: args.depend::<rooms::metadata::Service>(),
                state_accessor: args.depend::<rooms::state_accessor::Service>(),
                timeline: args.depend::<rooms::timeline::Service>(),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        loop {
            let retention_seconds = self
                .services
                .server
                .config
                .rooms
                .redaction_retention_seconds;

            if retention_seconds != 0 {
                debug_info!("Cleaning up retained events");

                let now = now_secs();

                let count = self
                    .db
                    .timeredacted_eventid
                    .keys::<(u64, &str)>()
                    .ready_try_take_while(|(time_redacted, _)| {
                        let time_redacted = *time_redacted;

                        Ok(expected!(time_redacted + retention_seconds) < now)
                    })
                    .ready_try_fold_default(|count: usize, (time_redacted, event_id)| {
                        self.db.eventid_originalpdu.remove(event_id)?;
                        self.db
                            .timeredacted_eventid
                            .del((time_redacted, event_id))?;

                        Ok(count.saturating_add(1))
                    })
                    .await?;

                debug_info!(?count, "Finished cleaning up retained events");
            }

            if self
                .services
                .server
                .config
                .rooms
                .retention_policy_enforcement
            {
                self.enforce_policies().await;
            }

            tokio::select! {
                () = tokio::time::sleep(SWEEP_INTERVAL) => {},
                () = self.services.server.until_shutdown() => return Ok(()),
            }
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Purges each room's history past the lifetime its `m.room.retention`
/// policy allows, raised to the configured minimum.
#[implement(Service)]
async fn enforce_policies(&self) {
    let min_lifetime_ms = self
        .services
        .server
        .config
        .rooms
        .retention_min_lifetime_secs
        .saturating_mul(1000);

    let rooms: Vec<OwnedRoomId> = self
        .services
        .metadata
        .iter_ids()
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for room_id in rooms {
        let Some(max_lifetime) = self.max_lifetime(&room_id).await else {
            continue;
        };

        let lifetime = max_lifetime.max(min_lifetime_ms);
        let cutoff = HistoryCutoff::Timestamp(now_millis().saturating_sub(lifetime));

        match self
            .services
            .timeline
            .purge_history(&room_id, cutoff, true)
            .await
        {
            Ok(purged) if purged.purged > 0 => {
                info!(%room_id, purged = purged.purged, "Enforced the room's retention policy");
            }
            Ok(_) => {}
            Err(e) => debug_warn!(%room_id, "Could not enforce the retention policy: {e}"),
        }
    }
}

#[implement(Service)]
async fn max_lifetime(&self, room_id: &RoomId) -> Option<u64> {
    self.services
        .state_accessor
        .room_state_get_content::<RetentionPolicy>(
            room_id,
            &StateEventType::from("m.room.retention"),
            "",
        )
        .await
        .ok()
        .and_then(|policy| policy.max_lifetime)
        .filter(|&lifetime| lifetime > 0)
}

#[implement(Service)]
pub async fn get_original_pdu(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db
        .eventid_originalpdu
        .get(event_id)
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn get_original_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db
        .eventid_originalpdu
        .get(event_id)
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn save_original_pdu(
    &self,
    event_id: &EventId,
    pdu: &CanonicalJsonObject,
    _state_lock: &RoomMutexGuard,
) {
    if !self.services.server.config.rooms.save_unredacted_events {
        return;
    }

    if self.db.eventid_originalpdu.exists(event_id).await.is_ok() {
        return;
    }

    let now = now_secs();

    self.db
        .eventid_originalpdu
        .raw_put(event_id, Json(pdu))
        .ok();

    self.db
        .timeredacted_eventid
        .put_raw((now, event_id), [])
        .ok();
}

#[implement(Service)]
pub fn retained_pdus_raw(&self) -> impl Stream<Item = Result<&[u8]>> + Send {
    self.db
        .eventid_originalpdu
        .raw_stream()
        .map_ok(|(_, pdu)| pdu)
}

#[implement(Service)]
pub fn purge_original(&self, event_id: &EventId) {
    self.db.eventid_originalpdu.remove(event_id).ok();
}
