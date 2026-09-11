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

#[implement(Service)]
pub async fn get_pdu_outlier(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db
        .eventid_outlierpdu
        .get(event_id)
        .await
        .deserialized()
}

#[implement(Service)]
pub async fn get_outlier_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db
        .eventid_outlierpdu
        .get(event_id)
        .await
        .deserialized()
}

#[implement(Service)]
#[tracing::instrument(skip(self, pdu), level = "debug")]
pub fn add_pdu_outlier(&self, event_id: &EventId, pdu: &CanonicalJsonObject) {
    self.db.eventid_outlierpdu.raw_put(event_id, Json(pdu)).ok();
}
