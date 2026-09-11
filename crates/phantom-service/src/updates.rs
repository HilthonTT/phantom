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

const LAST_SEEN_KEY: &[u8] = b"updates_last_seen_id";

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
        let interval = args.server.config.updates.check_for_updates_interval_s;

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
        if !self.services.server.config.updates.allow_check_for_updates {
            debug!("Checking for announcements is disabled by configuration");
            return Ok(());
        }

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

#[implement(Service)]
#[tracing::instrument(name = "updates", level = "debug", skip_all)]
async fn check(&self) -> Result {
    let url = &self.services.server.config.updates.check_for_updates_url;

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

#[implement(Service)]
async fn last_seen(&self) -> Option<u64> {
    self.db.global.get(LAST_SEEN_KEY).await.deserialized().ok()
}

#[implement(Service)]
fn set_last_seen(&self, id: u64) {
    self.db.global.raw_put(LAST_SEEN_KEY, id).ok();
}
