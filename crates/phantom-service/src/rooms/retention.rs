//! Originals of redacted events, and when to stop keeping them.
//!
//! Redaction is not deletion: the spec has the server strip an event down to
//! the fields its type requires and serve that stripped form from then on. The
//! bytes that were removed are still wanted for a while, though — a moderator
//! reviewing an abuse report needs to see what was actually said, and MSC2815
//! is the client API for asking. So a redaction copies the original PDU aside
//! before it rewrites the timeline copy.
//!
//! Keeping it aside forever would be a liability, so each copy is written with
//! the time it was retained and the worker here sweeps the ones that have
//! outlived `redaction_retention_seconds`. Two columns carry that: the
//! originals keyed by event id, and an index keyed by
//! `(retained_at, event_id)`. The index is what makes the sweep cheap — it is
//! ordered by time, so the worker walks it from the oldest entry and stops at
//! the first one still inside the window rather than reading every original to
//! find out how old it is.
//!
//! Both halves are optional. `save_unredacted_events` decides whether anything
//! is retained at all, and a `redaction_retention_seconds` of zero keeps what
//! is retained indefinitely.

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

/// How often the worker wakes to sweep expired originals.
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
            let retention_seconds = self.services.server.config.redaction_retention_seconds;

            if retention_seconds != 0 {
                debug_info!("Cleaning up retained events");

                let now = now_secs();

                // The index is ordered by retention time, so the walk stops at
                // the first entry still inside the window rather than reading
                // to the end of the column.
                let count = self
                    .db
                    .timeredacted_eventid
                    .keys::<(u64, &str)>()
                    .ready_try_take_while(|(time_redacted, _)| {
                        // Bound rather than dereferenced inline: `expected!`
                        // parses the expression as tokens and a leading `*`
                        // does not match its arithmetic pattern.
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

/// The retained original of a redacted event, parsed.
#[implement(Service)]
pub async fn get_original_pdu(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db
        .eventid_originalpdu
        .get(event_id)
        .await
        .deserialized()
}

/// The retained original as it was stored, rather than as a parsed PDU.
///
/// The canonical JSON is what the event was authenticated as — its signatures
/// are over these bytes — so a caller handing the original back out serves
/// what it read here rather than re-serializing the parsed form.
#[implement(Service)]
pub async fn get_original_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db
        .eventid_originalpdu
        .get(event_id)
        .await
        .deserialized()
}

/// Retains `pdu` as the original of an event about to be redacted.
///
/// Called from the redaction path with the room's state lock held, which is
/// what orders this against the timeline rewrite that follows. A second
/// redaction of the same event leaves the first original alone: it is the one
/// that has the content.
#[implement(Service)]
pub async fn save_original_pdu(
    &self,
    event_id: &EventId,
    pdu: &CanonicalJsonObject,
    _state_lock: &RoomMutexGuard,
) {
    if !self.services.server.config.save_unredacted_events {
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

/// Every retained original, as stored.
#[implement(Service)]
pub fn retained_pdus_raw(&self) -> impl Stream<Item = Result<&[u8]>> + Send {
    self.db
        .eventid_originalpdu
        .raw_stream()
        .map_ok(|(_, pdu)| pdu)
}

/// Drops the retained unredacted original of a purged event. The paired
/// `timeredacted_eventid` index entry is left for the retention worker to reap
/// at its scheduled time.
#[implement(Service)]
pub fn purge_original(&self, event_id: &EventId) {
    self.db.eventid_originalpdu.remove(event_id).ok();
}
