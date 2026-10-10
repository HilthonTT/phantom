//! Abuse reports users send about events, rooms and other users: stored for
//! the admin API, and posted into the admin room as they arrive.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use phantom_core::{Err, Result, debug_warn, implement, rand, stream::TryIgnore, time::now_millis};
use phantom_database::{Json, Map, table};
use ruma::{OwnedEventId, OwnedRoomId, OwnedUserId};
use serde::{Deserialize, Serialize};

use crate::{Dep, ops::admin};

pub struct Service {
    services: Services,
    reportid_report: Arc<Map>,
}

struct Services {
    admin: Dep<admin::Service>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Report {
    /// Sorts by when the report arrived.
    pub id: String,
    pub at_ms: u64,

    pub reporter: OwnedUserId,

    /// What was reported: an event in a room, a room, or a user.
    pub room_id: Option<OwnedRoomId>,
    pub event_id: Option<OwnedEventId>,
    pub user_id: Option<OwnedUserId>,

    pub reason: String,
}

impl Report {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match (&self.event_id, &self.room_id, &self.user_id) {
            (Some(_), ..) => "event",
            (None, Some(_), _) => "room",
            _ => "user",
        }
    }
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                admin: args.depend::<admin::Service>(),
            },
            reportid_report: args.db[table::REPORTID_REPORT].clone(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Stores a report and tells the admins of it in the admin room, with notice
/// as the message. A failed notice is logged; the report is kept either way.
#[implement(Service)]
pub async fn file(
    &self,
    reporter: OwnedUserId,
    room_id: Option<OwnedRoomId>,
    event_id: Option<OwnedEventId>,
    user_id: Option<OwnedUserId>,
    reason: String,
    notice: &str,
) -> Result<Report> {
    let at_ms = now_millis();
    let report = Report {
        id: format!("{at_ms:016}-{}", rand::string(6)),
        at_ms,
        reporter,
        room_id,
        event_id,
        user_id,
        reason,
    };

    self.reportid_report
        .raw_put(report.id.as_str(), Json(&report))?;

    if let Err(e) = self.services.admin.send_notice(notice).await {
        debug_warn!("Could not post the report to the admin room: {e}");
    }

    Ok(report)
}

/// Every open report, newest first.
#[implement(Service)]
pub async fn list(&self) -> Vec<Report> {
    let mut reports: Vec<Report> = self
        .reportid_report
        .raw_stream()
        .ignore_err()
        .filter_map(async |(_, value)| serde_json::from_slice(value).ok())
        .collect()
        .await;

    reports.reverse();
    reports
}

/// Closes a report, once an admin has dealt with it.
#[implement(Service)]
pub async fn dismiss(&self, id: &str) -> Result {
    if self.reportid_report.exists(id).await.is_err() {
        return Err!(Request(NotFound("There is no open report {id}.")));
    }

    self.reportid_report.remove(id)
}
