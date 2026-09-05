//! The announcement feed, and which announcements have already been seen.
//!
//! Some things an operator needs to know are not visible from inside their own
//! server: that a release fixed a vulnerability their version has, that a
//! config option they rely on is about to change meaning. So the project
//! publishes a feed of numbered announcements and a running server reads it
//! periodically, remembering the highest id it has read so an announcement is
//! surfaced once rather than every two hours.
//!
//! Two things about that are worth stating plainly, because both are choices
//! rather than accidents.
//!
//! **It is off by default.** A server that has not been asked to make an
//! outbound request does not make one, so `allow_check_for_updates` gates the
//! worker existing at all rather than gating the request inside it.
//!
//! **A first run announces nothing.** A fresh server has no high-water mark,
//! and reading the whole feed as unseen would greet its operator with every
//! notice the project ever published — the great majority about versions that
//! server never ran. So the first fetch records the newest id and says
//! nothing; from then on, everything past that mark is new.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use phantom_core::{Result, debug, err, implement, info, result::LogErr, server::Server, warn};
use phantom_database::{Deserialized, Map};
use serde::Deserialize;

use crate::{Dep, client};

pub struct Service {
    interval: Duration,
    db: Data,
    services: Services,
}

struct Data {
    global: Arc<Map>,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
}

/// The key the high-water mark is stored under in `global`.
const LAST_SEEN_KEY: &[u8] = b"updates_last_seen_id";

/// The feed as published.
///
/// Deserialized rather than read as loose JSON so a feed that has grown a
/// field this version does not understand is still read, and one whose shape
/// has changed incompatibly fails loudly instead of silently announcing
/// nothing.
#[derive(Debug, Deserialize)]
struct Feed {
    announcements: Vec<Announcement>,
}

#[derive(Debug, Deserialize)]
struct Announcement {
    id: u64,
    message: String,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let interval = args.server.config.check_for_updates_interval_s;

        Ok(Arc::new(Self {
            interval: Duration::from_secs(interval),
            db: Data {
                global: args.db["global"].clone(),
            },
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>("client"),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        if !self.services.server.config.allow_check_for_updates {
            debug!("Checking for announcements is disabled by configuration");
            return Ok(());
        }

        // A failed check is not a failed worker: the feed being unreachable is
        // an ordinary condition, and returning an error would have the manager
        // restart this service every 2.5 seconds for as long as the operator's
        // network is down.
        loop {
            self.check().await.log_err().ok();

            tokio::select! {
                () = tokio::time::sleep(self.interval) => {},
                () = self.services.server.until_shutdown() => return Ok(()),
            }
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Fetches the feed and logs whatever is newer than the high-water mark.
#[implement(Service)]
#[tracing::instrument(name = "updates", level = "debug", skip_all)]
async fn check(&self) -> Result {
    let url = &self.services.server.config.check_for_updates_url;

    let response = self
        .services
        .client
        .default
        .get(url)
        .send()
        .await
        .map_err(|e| err!(BadServerResponse("Failed to fetch {url}: {e}")))?
        .error_for_status()
        .map_err(|e| err!(BadServerResponse("Announcement feed {url} answered: {e}")))?
        .text()
        .await
        .map_err(|e| err!(BadServerResponse("Failed to read {url}: {e}")))?;

    let feed: Feed = serde_json::from_str(&response).map_err(|e| {
        err!(BadServerResponse(
            "Malformed announcement feed at {url}: {e}"
        ))
    })?;

    // The feed is published in ascending id order, but a published file is not
    // a database and a hand-edited one may not be. Taking the maximum rather
    // than the last entry means a feed out of order still advances the mark
    // past everything in it, so nothing is announced twice.
    let Some(newest) = feed.announcements.iter().map(|a| a.id).max() else {
        debug!("Announcement feed is empty");
        return Ok(());
    };

    let last_seen = self.last_seen().await;

    match last_seen {
        None => debug!(newest, "First check; recording the mark without announcing"),
        Some(last_seen) => {
            for announcement in feed
                .announcements
                .iter()
                .filter(|a| a.id > last_seen)
                .filter(|a| !a.message.trim().is_empty())
            {
                // Announcements are the operator's to act on, and a server
                // that logs them at `debug` has not told anyone. `warn` is
                // deliberate: the feed carries security notices.
                warn!(
                    id = announcement.id,
                    "Announcement: {}", announcement.message
                );
            }
        }
    }

    if last_seen != Some(newest) {
        self.set_last_seen(newest);
        info!(newest, "Announcement feed read");
    }

    Ok(())
}

/// The highest announcement id already surfaced, or `None` on a server that
/// has never read the feed.
#[implement(Service)]
async fn last_seen(&self) -> Option<u64> {
    self.db.global.get(LAST_SEEN_KEY).await.deserialized().ok()
}

#[implement(Service)]
fn set_last_seen(&self, id: u64) {
    self.db.global.raw_put(LAST_SEEN_KEY, id).ok();
}
