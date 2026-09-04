//! The server's own identity, its secrets, and the counter every event is
//! ordered by.
//!
//! What lives here is state the server has that belongs to no room and no
//! user: the account it posts as, the alias of its admin room, the secrets it
//! was configured with, and the event counter in [`counter`].
//!
//! Config values are *not* re-exported from here. A caller that wants one
//! reads `server.config` directly, so there is one place a setting is spelled
//! rather than a forwarding method per option that has to be kept in step.

pub mod counter;

use std::{
    collections::HashMap,
    fmt::Write,
    path::Path,
    sync::{Arc, RwLock},
    time::Instant,
};

use async_trait::async_trait;
use phantom_core::{Result, bytes::pretty, error, server::Server};
use ruma::{OwnedEventId, OwnedRoomAliasId, OwnedUserId, RoomAliasId, ServerName, UserId};

use self::counter::Counter;

pub struct Service {
    /// The monotonic counter every event is ordered by.
    pub counter: Counter,

    server: Arc<Server>,

    pub bad_event_ratelimiter: Arc<RwLock<HashMap<OwnedEventId, RateLimitState>>>,
    pub server_user: OwnedUserId,
    pub admin_alias: OwnedRoomAliasId,
    pub turn_secret: String,
    pub registration_token: Option<String>,
}

/// When a server last failed to serve an event, and how many times running.
type RateLimitState = (Instant, u32);

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;

        let turn_secret = read_secret(config.turn_secret_file.as_deref(), "TURN secret")
            .unwrap_or_else(|| config.turn_secret.clone());

        let registration_token = read_secret(
            config.registration_token_file.as_deref(),
            "registration token",
        )
        .or_else(|| config.registration_token.clone());

        Ok(Arc::new(Self {
            counter: Counter::new(&args),
            server: args.server.clone(),
            bad_event_ratelimiter: Arc::new(RwLock::new(HashMap::new())),
            admin_alias: OwnedRoomAliasId::try_from(format!("#admins:{}", args.server.name))
                .expect("#admins:server_name is valid alias name"),
            server_user: UserId::parse_with_server_name(String::from("phantom"), &args.server.name)
                .expect("@phantom:server_name is valid"),
            turn_secret,
            registration_token,
        }))
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        let (count, bytes) = self
            .bad_event_ratelimiter
            .read()
            .expect("locked for reading")
            .keys()
            .fold((0_usize, 0_usize), |(count, bytes), event_id| {
                (
                    count.saturating_add(1),
                    bytes
                        .saturating_add(event_id.as_str().len())
                        .saturating_add(size_of::<RateLimitState>()),
                )
            });

        writeln!(out, "bad_event_ratelimiter: {count} ({})", pretty(bytes))?;

        Ok(())
    }

    async fn clear_cache(&self) {
        self.bad_event_ratelimiter
            .write()
            .expect("locked for writing")
            .clear();
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Reads a secret held in a file beside the config, trimmed of the trailing
/// newline an editor leaves behind. `None` where there is no file to read or
/// it could not be read, which leaves the caller on its inline option.
fn read_secret(path: Option<&Path>, what: &str) -> Option<String> {
    let path = path?;

    std::fs::read_to_string(path)
        .inspect_err(|e| error!("Failed to read the {what} file {path:?}: {e}"))
        .ok()
        .map(|secret| secret.trim().to_owned())
}

impl Service {
    /// The next number in the event ordering. See [`counter`].
    #[inline]
    pub fn next_count(&self) -> Result<u64> {
        self.counter.next()
    }

    /// The last number handed out by [`Self::next_count`].
    #[inline]
    #[must_use]
    pub fn current_count(&self) -> u64 {
        self.counter.current()
    }

    #[inline]
    #[must_use]
    pub fn server_name(&self) -> &ServerName {
        self.server.name.as_ref()
    }

    /// Whether `user_id` is one of ours, decided by server name.
    #[inline]
    #[must_use]
    pub fn user_is_local(&self, user_id: &UserId) -> bool {
        self.server_is_ours(user_id.server_name())
    }

    #[inline]
    #[must_use]
    pub fn server_is_ours(&self, server_name: &ServerName) -> bool {
        server_name == self.server_name()
    }

    /// Whether `alias` names a room on this server, decided by server name.
    ///
    /// An alias from elsewhere is not ours to resolve out of the local column
    /// or to hand out, however familiar its localpart looks.
    #[inline]
    #[must_use]
    pub fn alias_is_local(&self, alias: &RoomAliasId) -> bool {
        self.server_is_ours(alias.server_name())
    }
}
