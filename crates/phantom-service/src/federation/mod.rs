mod execute;
pub mod feds;
mod peer;
mod rank;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use phantom_core::{
    Result, server::Server, time::exponential_backoff::exponential_backoff_streak_cap,
};
use phantom_database::Map;

pub use self::{
    peer::{Classification, PeerBackoff, ShouldAttempt},
    rank::{Candidates, WhenAllBackedOff},
};
use crate::{Dep, client, moderation, resolver, rooms, server_keys, server_state};

pub struct Service {
    services: Services,

    statuses: Arc<Map>,

    window_secs: u64,

    n_max: u32,

    grace: Duration,

    max_backoff: Duration,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
    moderation: Dep<moderation::Service>,
    resolver: Dep<resolver::Service>,
    server_keys: Dep<server_keys::Service>,
    server_state: Dep<server_state::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config.network;

        let window_secs = config.sender_timeout.max(1);
        let max_backoff = Duration::from_secs(config.sender_retry_backoff_limit);
        let n_max = exponential_backoff_streak_cap(Duration::from_secs(window_secs), max_backoff);

        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>("client"),
                moderation: args.depend::<moderation::Service>("moderation"),
                resolver: args.depend::<resolver::Service>("resolver"),
                server_keys: args.depend::<server_keys::Service>("server_keys"),
                server_state: args.depend::<server_state::Service>("server_state"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
            },
            statuses: args.db["servername_status"].clone(),
            window_secs,
            n_max,
            grace: Duration::from_secs(config.sender_retry_grace),
            max_backoff,
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}
