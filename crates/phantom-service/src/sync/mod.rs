//! What a client is waiting for, and what it has already been told.
//!
//! Sync is the endpoint a Matrix client spends its life in, and two things it
//! needs are awkward enough that neither belongs in the endpoint itself.
//!
//! **Waiting.** A client asks for everything new and, when there is nothing
//! new, expects the request to hang until there is. [`watch`] is that wait: it
//! parks on every database prefix the answer would be drawn from and returns
//! the moment any of them is written. Parking is what makes it correct as well
//! as cheap — a poll on a timer would either burn the server's time or add its
//! own interval to how long a message takes to arrive.
//!
//! **Remembering.** Sliding sync is defined as a conversation rather than a
//! request: a client sends the parts of its query that changed, and the server
//! answers against the whole query. That means the server holds the query, and
//! holds what it has already sent, per connection. [`connections`] is that
//! state.
//!
//! It is deliberately in memory and deliberately lost on restart. A client
//! whose connection the server has forgotten is told so — the response carries
//! no position it recognizes — and starts a new one, which is a round trip.
//! Persisting it would trade that round trip for a database write on every
//! sync from every device, which is the most frequent request a homeserver
//! serves.
//!
//! [`watch`]: Service::watch
//! [`connections`]: Service::connections

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
    /// The sliding sync conversations in progress, by user, device and the
    /// connection id the client chose.
    ///
    /// One lock over all of them rather than a lock per connection: what is
    /// held is a clone of a small struct, the critical section is a map lookup,
    /// and a per-connection lock would cost an allocation per device to save
    /// contention that a sync request's own frequency does not create.
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

/// The key a sliding sync conversation is held under.
///
/// A device may hold several at once — a client that syncs its room list and
/// its open room separately is the ordinary case — which is what the client's
/// own connection id distinguishes.
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
