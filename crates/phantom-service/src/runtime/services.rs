//! Every service the server is built out of, in one place.

use std::{
    collections::BTreeMap,
    fmt::Write,
    sync::{Arc, RwLock},
};

use phantom_core::{Result, debug, debug_info, info, server::Server, trace};
use phantom_database::Database;
use tokio::sync::Mutex;

use super::{
    contract::{Args, Service},
    manager::Manager,
    registry::{self, Map},
};
use crate::{
    account_data, client, config, key_backups, resolver, rooms, server_state, transaction_id,
};

/// Every service the server is built out of, and the database they share.
pub struct Services {
    pub client: Arc<client::Service>,
    pub config: Arc<config::Service>,
    pub resolver: Arc<resolver::Service>,
    pub rooms: rooms::Services,
    pub server_state: Arc<server_state::Service>,
    pub transaction_id: Arc<transaction_id::Service>,
    pub account_data: Arc<account_data::Service>,
    pub key_backups: Arc<key_backups::Service>,

    manager: Mutex<Option<Arc<Manager>>>,
    pub(crate) service: Arc<Map>,
    pub server: Arc<Server>,
    pub db: Arc<Database>,
}

impl Services {
    /// Opens the database and constructs every service against it.
    ///
    /// Nothing is running yet when this returns: a service's worker is only
    /// spawned by [`Self::start`], so that a service built early can depend on
    /// one built after it.
    ///
    /// Build order only matters where one service reaches another during its
    /// own construction rather than through a [`Dep`](super::Dep) — the
    /// resolver is built before the client for that reason, since every
    /// client is built against it.
    pub fn build(server: Arc<Server>) -> Result<Arc<Self>> {
        let db = Database::open(&server)?;
        let service: Arc<Map> = Arc::new(RwLock::new(BTreeMap::new()));

        macro_rules! build {
            ($tyname:ty) => {{
                let built = <$tyname>::build(Args {
                    db: &db,
                    server: &server,
                    service: &service,
                })?;

                registry::add(&service, built.clone(), built.clone());
                built
            }};
        }

        Ok(Arc::new(Self {
            resolver: build!(resolver::Service),
            client: build!(client::Service),
            config: build!(config::Service),
            rooms: rooms::Services {
                directory: build!(rooms::directory::Service),
                short: build!(rooms::short::Service),
                timeline: build!(rooms::timeline::Service),
            },
            server_state: build!(server_state::Service),
            transaction_id: build!(transaction_id::Service),
            account_data: build!(account_data::Service),
            key_backups: build!(key_backups::Service),

            manager: Mutex::new(None),
            service,
            server,
            db,
        }))
    }

    /// Starts the manager, and through it every service's worker.
    pub async fn start(self: &Arc<Self>) -> Result<Arc<Self>> {
        debug_info!("Starting services...");

        let manager = Manager::new(self);
        self.manager.lock().await.replace(manager.clone());

        // Outside the lock: starting is what spawns the workers, and it is
        // long enough that holding the handle for it would block a concurrent
        // `stop` from ever being able to take it.
        manager.start().await?;

        debug_info!("Services startup complete.");
        Ok(Arc::clone(self))
    }

    /// Resolves once the manager has finished, which is once every worker has.
    pub async fn poll(&self) -> Result<()> {
        if let Some(manager) = self.manager.lock().await.as_ref() {
            return manager.poll().await;
        }

        Ok(())
    }

    /// Interrupts every service and waits for the manager to wind down.
    pub async fn stop(&self) {
        info!("Shutting down services...");

        // Interrupting first is what lets a worker return from its loop rather
        // than be waited on until it happens to finish.
        self.interrupt();

        if let Some(manager) = self.manager.lock().await.take() {
            manager.stop().await;
        }

        debug_info!("Services shutdown complete.");
    }

    /// Drops whatever runtime state the services are holding on to.
    pub async fn clear_cache(&self) {
        for service in self.services() {
            service.clear_cache().await;
        }
    }

    /// A markdown report of what the services and the database are holding.
    pub async fn memory_usage(&self) -> Result<String> {
        let mut out = String::new();

        for service in self.services() {
            service.memory_usage(&mut out).await?;
        }

        write!(out, "\n{}", self.db.engine.memory_usage()?)?;

        Ok(out)
    }

    /// Tells every service to return from its worker loop.
    fn interrupt(&self) {
        debug!("Interrupting services...");

        for (name, (service, ..)) in &*self.service.read().expect("locked for reading") {
            if let Some(service) = service.upgrade() {
                trace!("Interrupting {name}");
                service.interrupt();
            }
        }
    }

    /// The services still alive, in name order.
    ///
    /// Collected rather than borrowed: the callers await between services, and
    /// the read guard cannot be held across that. The map holds weak
    /// references so that services may point at each other, so a name whose
    /// service has already dropped is simply skipped.
    fn services(&self) -> Vec<Arc<dyn Service>> {
        self.service
            .read()
            .expect("locked for reading")
            .values()
            .filter_map(|(service, ..)| service.upgrade())
            .collect()
    }
}
