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
    account_data, admin, appservice, client, config, emergency, federation, key_backups, media,
    moderation, presence, pusher, resolver, rooms, sending, server_keys, server_state, sync,
    transaction_id, updates, users,
};

/// Every service the server is built out of, and the database they share.
pub struct Services {
    pub client: Arc<client::Service>,
    pub config: Arc<config::Service>,
    pub media: Arc<media::Service>,
    pub moderation: Arc<moderation::Service>,
    pub resolver: Arc<resolver::Service>,
    pub federation: Arc<federation::Service>,
    pub rooms: rooms::Services,
    pub server_keys: Arc<server_keys::Service>,
    pub server_state: Arc<server_state::Service>,
    pub sync: Arc<sync::Service>,
    pub transaction_id: Arc<transaction_id::Service>,
    pub account_data: Arc<account_data::Service>,
    pub key_backups: Arc<key_backups::Service>,
    pub appservice: Arc<appservice::Service>,
    pub users: Arc<users::Service>,
    pub emergency: Arc<emergency::Service>,
    pub presence: Arc<presence::Service>,
    pub pusher: Arc<pusher::Service>,
    pub sending: Arc<sending::Service>,
    pub admin: Arc<admin::Service>,
    pub updates: Arc<updates::Service>,

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
            media: build!(media::Service),
            moderation: build!(moderation::Service),
            federation: build!(federation::Service),
            rooms: rooms::Services {
                alias: build!(rooms::alias::Service),
                directory: build!(rooms::directory::Service),
                event_handler: build!(rooms::event_handler::Service),
                short: build!(rooms::short::Service),
                spaces: build!(rooms::spaces::Service),
                state: build!(rooms::state::Service),
                state_accessor: build!(rooms::state_accessor::Service),
                state_cache: build!(rooms::state_cache::Service),
                state_compressor: build!(rooms::state_compressor::Service),
                search: build!(rooms::search::Service),
                read_receipt: build!(rooms::read_receipt::Service),
                timeline: build!(rooms::timeline::Service),
                auth_chain: build!(rooms::auth_chain::Service),
                lazy_loading: build!(rooms::lazy_loading::Service),
                metadata: build!(rooms::metadata::Service),
                outlier: build!(rooms::outlier::Service),
                pdu_metadata: build!(rooms::pdu_metadata::Service),
                threads: build!(rooms::threads::Service),
                typing: build!(rooms::typing::Service),
                user: build!(rooms::user::Service),
            },
            server_keys: build!(server_keys::Service),
            server_state: build!(server_state::Service),
            sync: build!(sync::Service),
            transaction_id: build!(transaction_id::Service),
            account_data: build!(account_data::Service),
            key_backups: build!(key_backups::Service),
            appservice: build!(appservice::Service),
            users: build!(users::Service),
            emergency: build!(emergency::Service),
            presence: build!(presence::Service),
            pusher: build!(pusher::Service),
            sending: build!(sending::Service),
            admin: build!(admin::Service),
            updates: build!(updates::Service),

            manager: Mutex::new(None),
            service,
            server,
            db,
        }))
    }

    /// Starts the manager, and through it every service's worker.
    pub async fn start(self: &Arc<Self>) -> Result<Arc<Self>> {
        debug_info!("Starting services...");

        self.admin.set_services(Some(self));

        let manager = Manager::new(self);
        self.manager.lock().await.replace(manager.clone());

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

        self.interrupt();

        if let Some(manager) = self.manager.lock().await.take() {
            manager.stop().await;
        }

        self.admin.set_services(None);

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
