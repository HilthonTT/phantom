use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{Stream, TryStreamExt};
use phantom_core::{
    Result, debug_info, expected, implement, matrix::pdu::PduEvent, server::Server,
    stream::TryReadyExt, time::now_secs,
};
use phantom_database::{Deserialized, Json, Map};
use ruma::{CanonicalJsonObject, EventId};

use crate::rooms::state::RoomMutexGuard;

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
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                eventid_originalpdu: args.db["eventid_originalpdu"].clone(),
                timeredacted_eventid: args.db["timeredacted_eventid"].clone(),
            },
            services: Services {
                server: args.server.clone(),
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
