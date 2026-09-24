pub mod counter;

use std::{
    collections::HashMap,
    fmt::Write,
    sync::{Arc, RwLock},
    time::Instant,
};

use async_trait::async_trait;
use phantom_core::{Result, bytes::pretty, secret, server::Server};
use ruma::{OwnedEventId, OwnedRoomAliasId, OwnedUserId, RoomAliasId, ServerName, UserId};

use self::counter::Counter;

pub struct Service {
    pub counter: Counter,

    server: Arc<Server>,

    pub bad_event_ratelimiter: Arc<RwLock<HashMap<OwnedEventId, RateLimitState>>>,
    pub server_user: OwnedUserId,
    pub admin_alias: OwnedRoomAliasId,
    pub turn_secret: String,
    pub registration_token: Option<String>,
}

type RateLimitState = (Instant, u32);

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;

        let turn_secret = secret::resolve(
            config.turn.turn_secret_file.as_deref(),
            Some(config.turn.turn_secret.as_str()),
            "TURN secret",
        )
        .unwrap_or_default();

        let registration_token = secret::resolve(
            config.auth.registration_token_file.as_deref(),
            config.auth.registration_token.as_deref(),
            "registration token",
        );

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

impl Service {
    #[inline]
    pub fn next_count(&self) -> Result<u64> {
        self.counter.next()
    }

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

    #[inline]
    #[must_use]
    pub fn alias_is_local(&self, alias: &RoomAliasId) -> bool {
        self.server_is_ours(alias.server_name())
    }
}
