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
    flattened = "auth rooms media database logging federation network dns presence turn admin oidc rendezvous updates",
    ignore = "catchall smtp identity_provider auth rooms media database logging federation network dns presence turn admin oidc rendezvous updates"
)]
pub struct Config {
    pub server_name: String,

    #[serde(default = "default_address")]
    address: ListeningAddr,

    #[serde(default = "default_port")]
    port: ListeningPort,

    #[serde(default = "default_notification_push_path")]
    pub notification_push_path: String,

    #[serde(default)]
    pub identity_provider: BTreeMap<String, IdentityProvider>,

    #[serde(default)]
    pub smtp: SmtpConfig,

    #[serde(flatten)]
    pub catchall: figment::value::Dict,

    #[serde(flatten)]
    pub auth: auth::Auth,

    #[serde(flatten)]
    pub rooms: rooms::Rooms,

    #[serde(flatten)]
    pub media: media::Media,

    #[serde(flatten)]
    pub database: database::Database,

    #[serde(flatten)]
    pub logging: logging::Logging,

    #[serde(flatten)]
    pub federation: federation::Federation,

    #[serde(flatten)]
    pub network: network::Network,

    #[serde(flatten)]
    pub dns: dns::Dns,

    #[serde(flatten)]
    pub presence: presence::Presence,

    #[serde(flatten)]
    pub turn: turn::Turn,

    #[serde(flatten)]
    pub admin: admin::Admin,

    #[serde(flatten)]
    pub oidc: oidc::Oidc,

    #[serde(flatten)]
    pub rendezvous: rendezvous::Rendezvous,

    #[serde(flatten)]
    pub updates: updates::Updates,
}

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
pub mod rendezvous;
pub mod rooms;
pub mod turn;
pub mod updates;

pub mod identity;
pub mod smtp;
pub mod storage;

impl Config {
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

    pub fn new(raw_config: &Figment) -> Result<Self> {
        let config = raw_config
            .extract::<Self>()
            .map_err(|error| err!("There was a problem with your configuration file: {error}"))?;

        validate(&config)?;

        Ok(config)
    }

    #[must_use]
    pub fn media_path(&self) -> PathBuf {
        self.media
            .media_path
            .clone()
            .unwrap_or_else(|| self.database.database_path.join("media"))
    }

    pub fn log_filter(&self) -> Result<EnvFilter> {
        EnvFilter::builder()
            .with_regex(self.logging.log_filter_regex)
            .parse(&self.logging.log)
            .map_err(|error| err!(Config("log", "{error}")))
    }

    pub fn span_events(&self) -> Result<FmtSpan> {
        fmt_span::from_str(&self.logging.log_span_events)
            .map_err(|error| err!(Config("log_span_events", "{error}")))
    }

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

const DEPRECATED_KEYS: &[&str] = &[];

#[cfg(test)]
mod tests;
