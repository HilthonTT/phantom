//! The admin audit log: one record for every change an admin made through
//! the admin API or an `!admin` command, kept in the database so it outlives
//! a restart.
//!
//! Records are written by the admin API's audit layer and the command
//! processor, never by the handlers themselves, so an admin route or command
//! added later is recorded without having to remember to.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use phantom_core::{Result, rand, stream::TryIgnore, time::now_millis};
use phantom_database::{Json, Map, table};
use serde::{Deserialize, Serialize};

pub struct Service {
    auditid_record: Arc<Map>,
}

/// One admin action and how it went.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    /// Sorts by when the action was taken; the cursor a page of the log is
    /// asked for by.
    pub id: String,
    pub at_ms: u64,

    /// The admin who acted.
    pub admin: String,

    /// "api" for the admin API, "command" for an `!admin` command.
    pub via: String,

    /// What was done: the method and route for the API, the subcommand for a
    /// command.
    pub action: String,

    /// What it was done to, as named in the request; empty for an action on
    /// the server as a whole.
    pub target: String,

    pub ok: bool,

    /// The status the action answered with, and the error when it failed.
    pub result: String,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            auditid_record: args.db[table::AUDITID_RECORD].clone(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Appends a record, stamping its time and id.
    pub fn record(
        &self,
        admin: &str,
        via: &str,
        action: &str,
        target: &str,
        ok: bool,
        result: &str,
    ) -> Result<Record> {
        let at_ms = now_millis();
        let record = Record {
            id: format!("{at_ms:016}-{}", rand::string(6)),
            at_ms,
            admin: admin.to_owned(),
            via: via.to_owned(),
            action: action.to_owned(),
            target: target.to_owned(),
            ok,
            result: result.to_owned(),
        };

        self.auditid_record
            .raw_put(record.id.as_str(), Json(&record))?;

        Ok(record)
    }

    /// Up to limit records, newest first, starting after the record before
    /// names when it is given.
    pub async fn page(&self, before: Option<&str>, limit: usize) -> Vec<Record> {
        let stream = match before {
            Some(before) => self.auditid_record.rev_raw_stream_from(before).boxed(),
            None => self.auditid_record.rev_raw_stream().boxed(),
        };

        stream
            .ignore_err()
            .filter(|(key, _)| {
                std::future::ready(before.is_none_or(|before| *key != before.as_bytes()))
            })
            .filter_map(async |(_, value)| serde_json::from_slice(value).ok())
            .take(limit)
            .collect()
            .await
    }
}
