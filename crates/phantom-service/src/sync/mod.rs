mod connection;
mod watch;

use std::{
    collections::BTreeMap,
    fmt::Write,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use phantom_core::{Result, server::Server};
use phantom_database::Database;
use ruma::{OwnedDeviceId, OwnedUserId};

pub use self::connection::{Connection, ConnectionKey};
use crate::{Dep, rooms, users};

pub struct Service {
    connections: Mutex<BTreeMap<ConnectionKey, Connection>>,

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
            services: Services {
                server: args.server.clone(),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                typing: args.depend::<rooms::typing::Service>("rooms::typing"),
                users: args.depend::<users::Service>("users"),
            },
            db: args.db.clone(),
        }))
    }

    async fn clear_cache(&self) {
        self.connections.lock().expect("locked").clear();
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        let connections = self.connections.lock().expect("locked");

        let known_rooms: usize = connections
            .values()
            .map(|connection| {
                connection
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
    user_id: &OwnedUserId,
    device_id: &OwnedDeviceId,
    conn_id: Option<&str>,
) -> ConnectionKey {
    (
        user_id.clone(),
        device_id.clone(),
        conn_id.unwrap_or_default().to_owned(),
    )
}
