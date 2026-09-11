//! Server configuration.
//!
//! Every field below is a config option: its doc comment is the documentation
//! users read, and `#[config_example_generator]` turns this struct into
//! `phantom-example.toml` at the workspace root on every `cargo build`. Edit
//! the docs here, never that file — it is regenerated and overwritten.

mod defaults;
mod listen;
pub mod manager;
mod prelude;
pub mod proxy;
pub mod validate;

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
};

use bytesize::ByteSize;
use either::{
    Either,
    Either::{Left, Right},
};
use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use phantom_macros::config_example_generator;
use ruma::OwnedServerName;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

use self::{
    defaults::*,
    listen::{ListeningAddr, ListeningPort},
};
pub use self::{
    identity::IdentityProvider,
    listen::IpLookupStrategy,
    manager::Manager,
    proxy::ProxyConfig,
    smtp::SmtpConfig,
    storage::{StorageProvider, StorageProviderLocal, StorageProviderS3},
    validate::validate,
};
use crate::{Result, err, log::fmt_span};

/// All the config options for phantom.
#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global",
    undocumented = "# This item is undocumented. Please contribute documentation for it.",
    header = r#"### phantom configuration
###
### THIS FILE IS GENERATED. CHANGES/CONTRIBUTIONS IN THE REPO WILL BE
### OVERWRITTEN!
###
### You should rename this file before configuring your server. Changes to
### documentation and defaults can be contributed in source code at
### crates/phantom-core/src/config/mod.rs. This file is generated when
### building.
###
### Any values pre-populated are the default values for said config option.
"#,
    flattened = "auth rooms media database logging federation network dns presence turn admin oidc updates",
    ignore = "catchall smtp identity_provider auth rooms media database logging federation network dns presence turn admin oidc updates"
)]
pub struct Config {
    /// The server_name is the pretty name of this server. It is used as a
    /// suffix for user and room IDs/aliases.
    ///
    /// YOU NEED TO EDIT THIS. THIS CANNOT BE CHANGED AFTER WITHOUT A DATABASE
    /// WIPE.
    ///
    /// example: "phantom.chat"
    pub server_name: String,

    /// The default address (IPv4 or IPv6) phantom will listen on.
    ///
    /// To listen on multiple addresses, specify a vector e.g. ["127.0.0.1",
    /// "::1"]
    ///
    /// default: ["127.0.0.1", "::1"]
    #[serde(default = "default_address")]
    address: ListeningAddr,

    /// The port(s) phantom will listen on.
    ///
    /// To listen on multiple ports, specify a vector e.g. [8080, 8448]
    ///
    /// default: 8008
    #[serde(default = "default_port")]
    port: ListeningPort,

    /// Path a push gateway is expected to serve its notify endpoint at. Only
    /// the appservice-style pushers that do not carry their own URL use it.
    ///
    /// default: "/_matrix/push/v1/notify"
    #[serde(default = "default_notification_push_path")]
    pub notification_push_path: String,

    /// The identity providers users may authorize against, keyed by a name of
    /// the operator's choosing.
    ///
    /// Each entry is one OAuth application registered with an upstream
    /// provider. The key names the section and is not otherwise used — a
    /// provider is identified by its `client_id`, or by its `brand` where only
    /// one provider carries that brand.
    // external structure; separate section
    #[serde(default)]
    pub identity_provider: BTreeMap<String, IdentityProvider>,

    /// Configures outbound SMTP email delivery.
    ///
    /// Providing a connection URI enables the email subsystem. Registration
    /// flags determine when a verified address is required.
    // external structure; separate section
    #[serde(default)]
    pub smtp: SmtpConfig,

    /// Any TOML key phantom does not recognise lands here rather than failing
    /// deserialization, so unknown options can be reported rather than
    /// silently ignored.
    #[serde(flatten)]
    pub catchall: figment::value::Dict,

    /// Registration, login tokens, and what a new account starts with. See [`auth`](self::auth).
    #[serde(flatten)]
    pub auth: auth::Auth,

    /// Rooms: what may be created, and what is kept. See [`rooms`](self::rooms).
    #[serde(flatten)]
    pub rooms: rooms::Rooms,

    /// Uploaded files, thumbnails, and URL previews. See [`media`](self::media).
    #[serde(flatten)]
    pub media: media::Media,

    /// The database: where it lives, how much it caches, and RocksDB itself. See [`database`](self::database).
    #[serde(flatten)]
    pub database: database::Database,

    /// What the server logs, and how. See [`logging`](self::logging).
    #[serde(flatten)]
    pub logging: logging::Logging,

    /// Talking to other homeservers. See [`federation`](self::federation).
    #[serde(flatten)]
    pub federation: federation::Federation,

    /// Outbound HTTP: timeouts, connection pools, and compression. See [`network`](self::network).
    #[serde(flatten)]
    pub network: network::Network,

    /// Name resolution. See [`dns`](self::dns).
    #[serde(flatten)]
    pub dns: dns::Dns,

    /// Presence, read receipts, and typing notifications. See [`presence`](self::presence).
    #[serde(flatten)]
    pub presence: presence::Presence,

    /// The TURN server clients are handed for voice and video. See [`turn`](self::turn).
    #[serde(flatten)]
    pub turn: turn::Turn,

    /// The admin room and the commands it accepts. See [`admin`](self::admin).
    #[serde(flatten)]
    pub admin: admin::Admin,

    /// OpenID Connect login. See [`oidc`](self::oidc).
    #[serde(flatten)]
    pub oidc: oidc::Oidc,

    /// Checking whether a newer phantom has been released. See [`updates`](self::updates).
    #[serde(flatten)]
    pub updates: updates::Updates,
}

// Declared here, between `Config` and the structs that open their own
// sections, because the example generator writes the file as a side effect
// of macro expansion: everything continuing `[global]` has to expand after
// `Config` opens it and before the first subsection is opened.

pub mod admin;
pub mod auth;
pub mod database;
pub mod dns;
pub mod federation;
pub mod logging;
pub mod media;
pub mod network;
pub mod oidc;
pub mod presence;
pub mod rooms;
pub mod turn;
pub mod updates;

// These open sections of their own, so they come after everything that
// continues `[global]`.
pub mod identity;
pub mod smtp;
pub mod storage;

impl Config {
    /// Layers the given config files and the `PHANTOM_` environment onto one
    /// [`Figment`], without yet checking that the result is a valid [`Config`].
    pub fn load<'a, I>(paths: I) -> Result<Figment>
    where
        I: Iterator<Item = &'a Path>,
    {
        let config = Env::var("PHANTOM_CONFIG")
            .into_iter()
            .map(Toml::file)
            .chain(paths.map(Toml::file))
            .fold(Figment::new(), |config, file| config.merge(file.nested()))
            .merge(Env::prefixed("PHANTOM_").global().split("__"));

        Ok(config)
    }

    /// Extracts and validates the config produced by [`Self::load`].
    pub fn new(raw_config: &Figment) -> Result<Self> {
        let config = raw_config
            .extract::<Self>()
            .map_err(|error| err!("There was a problem with your configuration file: {error}"))?;

        validate(&config)?;

        Ok(config)
    }

    /// Where media files are kept, resolved.
    ///
    /// `media_path` where an operator set one, and a `media` directory beside
    /// the database otherwise.
    #[must_use]
    pub fn media_path(&self) -> PathBuf {
        self.media
            .media_path
            .clone()
            .unwrap_or_else(|| self.database.database_path.join("media"))
    }

    /// The console layer's filter, built from `log` and `log_filter_regex`.
    ///
    /// Lives here rather than at the logging callsite so that [`validate`] can
    /// reject a malformed filter while the config is being loaded, instead of
    /// the server starting with a filter it silently fell back to.
    pub fn log_filter(&self) -> Result<EnvFilter> {
        EnvFilter::builder()
            .with_regex(self.logging.log_filter_regex)
            .parse(&self.logging.log)
            .map_err(|error| err!(Config("log", "{error}")))
    }

    /// The span lifecycle points to log, from `log_span_events`.
    pub fn span_events(&self) -> Result<FmtSpan> {
        fmt_span::from_str(&self.logging.log_span_events)
            .map_err(|error| err!(Config("log_span_events", "{error}")))
    }

    /// Every `address` × `port` pair the server should bind.
    #[must_use]
    pub fn get_bind_addrs(&self) -> Vec<SocketAddr> {
        let hosts = self.get_bind_hosts();
        let ports = self.get_bind_ports();

        let mut addrs = Vec::with_capacity(hosts.len().saturating_mul(ports.len()));
        for host in &hosts {
            for port in &ports {
                addrs.push(SocketAddr::new(*host, *port));
            }
        }

        addrs
    }

    fn get_bind_hosts(&self) -> Vec<IpAddr> {
        match &self.address.addrs {
            Left(addr) => vec![*addr],
            Right(addrs) => addrs.clone(),
        }
    }

    fn get_bind_ports(&self) -> Vec<u16> {
        match &self.port.ports {
            Left(port) => vec![*port],
            Right(ports) => ports.clone(),
        }
    }
}

/// Config options that older versions of phantom accepted. They are still
/// parsed into `catchall` so that `validate` can name them, rather than being
/// reported as unknown.
const DEPRECATED_KEYS: &[&str] = &[];

#[cfg(test)]
mod tests;
