mod connection;
mod watch;

#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fmt::Write,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use phantom_core::{Result, debug, runtime::server::Server, time::now_millis};
use phantom_database::{Database, Map, table};
use ruma::{DeviceId, UserId};
use tokio::{sync::Notify, time::sleep};

use self::connection::Cached;
pub use self::{
    connection::{Connection, ConnectionKey},
    watch::Watch,
};
use crate::{Dep, accounts::users, rooms};

/// How often the worker deletes expired sliding sync connections.
const EXPIRY_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub struct Service {
    /// Sliding sync connections served since startup, in front of
    /// `slidingsync`, which holds every connection across restarts.
    connections: Mutex<BTreeMap<ConnectionKey, Cached>>,
    slidingsync: Arc<Map>,
    interrupt: Notify,

    services: Services,
    db: Arc<Database>,
}

struct Services {
    server: Arc<Server>,
    short: Dep<rooms::short::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    typing: Dep<rooms::typing::Service>,
    users: Dep<users::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            connections: Mutex::new(BTreeMap::new()),
            slidingsync: args.db[table::USERDEVICECONNID_SLIDINGSYNC].clone(),
            interrupt: Notify::new(),
            services: Services {
                server: args.server.clone(),
                short: args.depend::<rooms::short::Service>(),
                state_cache: args.depend::<rooms::state_cache::Service>(),
                typing: args.depend::<rooms::typing::Service>(),
                users: args.depend::<users::Service>(),
            },
            db: args.db.clone(),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        if self.services.server.config.database.rocksdb_read_only {
            return Ok(());
        }

        loop {
            let expired = self.expire_connections(now_millis()).await;
            if expired > 0 {
                debug!(expired, "Deleted expired sliding sync connections");
            }

            tokio::select! {
                () = self.interrupt.notified() => break,
                () = sleep(EXPIRY_SWEEP_INTERVAL) => {},
            }
        }

        Ok(())
    }

    fn interrupt(&self) {
        self.interrupt.notify_one();
    }

    async fn clear_cache(&self) {
        self.connections.lock().expect("locked").clear();
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        let connections = self.connections.lock().expect("locked");

        let known_rooms: usize = connections
            .values()
            .map(|cached| {
                cached
                    .connection
                    .known_rooms
                    .values()
                    .map(BTreeMap::len)
                    .sum::<usize>()
            })
            .sum();

        writeln!(out, "sliding_sync_connections: {}", connections.len())?;
        writeln!(out, "sliding_sync_known_rooms: {known_rooms}")?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

pub(crate) fn connection_key(
    user_id: &UserId,
    device_id: &DeviceId,
    conn_id: Option<&str>,
) -> ConnectionKey {
    (
        user_id.to_owned(),
        device_id.to_owned(),
        conn_id.unwrap_or_default().to_owned(),
    )
}
