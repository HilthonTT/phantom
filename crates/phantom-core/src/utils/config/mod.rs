//! Server configuration.
//!
//! Every field below is a config option: its doc comment is the documentation
//! users read, and `#[config_example_generator]` turns this struct into
//! `phantom-example.toml` at the workspace root on every `cargo build`. Edit
//! the docs here, never that file — it is regenerated and overwritten.

pub mod check;
pub mod manager;
pub mod proxy;

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
};

use either::{
    Either,
    Either::{Left, Right},
};
use figment::{
    Figment,
    providers::{Env, Format, Toml},
};
use phantom_macros::config_example_generator;
use serde::Deserialize;

pub use self::check::check;
use crate::{Result, err};

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
### crates/phantom-core/src/utils/config/mod.rs. This file is generated when
### building.
###
### Any values pre-populated are the default values for said config option.
"#,
    ignore = "catchall"
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

    /// Path to the directory holding the database.
    ///
    /// YOU NEED TO EDIT THIS.
    ///
    /// example: "/var/lib/phantom"
    pub database_path: PathBuf,

    /// Enable the built-in metrics endpoint.
    #[serde(default)]
    pub allow_metrics: bool,

    /// A shared secret required to register an account.
    ///
    /// display: sensitive
    pub registration_token: Option<String>,

    /// Any TOML key phantom does not recognise lands here rather than failing
    /// deserialization, so unknown options can be reported rather than
    /// silently ignored.
    #[serde(flatten)]
    pub catchall: figment::value::Dict,
}

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

        config.check()?;

        Ok(config)
    }

    pub fn check(&self) -> Result {
        check(self)
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

/// Accepts either a single address or a list of them.
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
struct ListeningAddr {
    #[serde(with = "either::serde_untagged")]
    addrs: Either<IpAddr, Vec<IpAddr>>,
}

/// Accepts either a single port or a list of them.
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
struct ListeningPort {
    #[serde(with = "either::serde_untagged")]
    ports: Either<u16, Vec<u16>>,
}

/// Config options that older versions of phantom accepted. They are still
/// parsed into `catchall` so that `check` can name them, rather than being
/// reported as unknown.
const DEPRECATED_KEYS: &[&str] = &[];

fn default_address() -> ListeningAddr {
    ListeningAddr {
        addrs: Right(vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]),
    }
}

fn default_port() -> ListeningPort {
    ListeningPort { ports: Left(8008) }
}

#[cfg(test)]
mod tests {
    use figment::providers::Toml;

    use super::*;

    fn config(toml: &str) -> Result<Config> {
        Config::new(&Figment::new().merge(Toml::string(toml).nested()))
    }

    #[test]
    fn defaults_apply_and_bind_addrs_cross_product() {
        let config = config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            port = [8008, 8448]
            "#,
        )
        .expect("config is valid");

        assert_eq!(
            config.get_bind_addrs().len(),
            4,
            "2 default addrs x 2 ports"
        );
        assert!(!config.allow_metrics, "serde default");
    }

    #[test]
    fn unknown_keys_land_in_catchall() {
        let config = config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            not_a_real_option = 5
            "#,
        )
        .expect("config is valid");

        assert!(config.catchall.contains_key("not_a_real_option"));
    }

    #[test]
    fn display_masks_sensitive_and_lists_fields() {
        let config = config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            registration_token = "hunter2"
            "#,
        )
        .expect("config is valid");

        let rendered = config.to_string();
        assert!(rendered.contains("| server_name | \"phantom.chat\" |"));
        assert!(rendered.contains("| registration_token | *********** |"));
        assert!(!rendered.contains("hunter2"), "secret must not be rendered");
        assert!(
            !rendered.contains("catchall"),
            "ignored field is not a config option"
        );
    }

    #[test]
    fn missing_required_option_is_an_error() {
        assert!(config("[global]\nserver_name = \"phantom.chat\"\n").is_err());
    }
}
