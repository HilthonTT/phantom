//! Events accepted but not yet placed in a room's timeline.
//!
//! An outlier is an event the server has validated and stored without knowing
//! where it belongs: it arrived referencing state the server has not caught up
//! to, usually as part of an auth chain fetched over federation. It stays here
//! until the timeline it belongs to reaches it, at which point the timeline
//! takes ownership and this copy is dropped.

use std::sync::Arc;

use phantom_core::{Result, implement, matrix::pdu::PduEvent};
use phantom_database::{Deserialized, Json, Map};
use ruma::{CanonicalJsonObject, EventId};

pub struct Service {
    db: Data,
}

struct Data {
    eventid_outlierpdu: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                eventid_outlierpdu: args.db["eventid_outlierpdu"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Returns the pdu from the outlier tree.
#[implement(Service)]
pub async fn get_pdu_outlier(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db
        .eventid_outlierpdu
        .get(event_id)
        .await
        .deserialized()
}

/// The outlier as it was stored, rather than as a parsed PDU.
///
/// The canonical JSON is what an event is authenticated as — its signatures
/// are over these bytes — so a caller putting the event in a room writes back
/// what it read here rather than re-serializing the parsed form.
#[implement(Service)]
pub async fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db
        .eventid_outlierpdu
        .get(event_id)
        .await
        .deserialized()
}

/// Append the PDU as an outlier.
#[implement(Service)]
#[tracing::instrument(skip(self, pdu), level = "debug")]
pub fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) {
    self.db.eventid_outlierpdu.raw_put(event_id, Json(pdu)).ok();
}
