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
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

pub use self::{check::check, manager::Manager, proxy::ProxyConfig};
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

    /// Max log level for phantom. Allows debug, info, warn, or error.
    ///
    /// See also:
    /// https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives
    ///
    /// **Caveat**:
    /// For release builds, the tracing crate is configured to only implement
    /// levels higher than error to avoid unnecessary overhead in the compiled
    /// binary from trace macros. For debug builds, this restriction is not
    /// applied.
    ///
    /// default: "info"
    #[serde(default = "default_log")]
    pub log: String,

    /// Output logs with ANSI colours. Colours are omitted regardless of this
    /// setting when running under systemd, where they would be stored verbatim
    /// in the journal.
    ///
    /// default: true
    #[serde(default = "true_fn", alias = "log_colours")]
    pub log_colors: bool,

    /// Configures the span events which will be outputted with the log.
    ///
    /// Accepts one or more of "new", "enter", "exit", "close", "active",
    /// "full" or "none", separated by commas.
    ///
    /// default: "none"
    #[serde(default = "default_log_span_events")]
    pub log_span_events: String,

    /// Configures whether `log` matches values using regular expressions. See
    /// the tracing_subscriber documentation on Directives.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub log_filter_regex: bool,

    /// Toggles the display of ThreadId in tracing log output.
    ///
    /// default: false
    #[serde(default)]
    pub log_thread_ids: bool,

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

    /// The console layer's filter, built from `log` and `log_filter_regex`.
    ///
    /// Lives here rather than at the logging callsite so that [`check`] can
    /// reject a malformed filter while the config is being loaded, instead of
    /// the server starting with a filter it silently fell back to.
    pub fn log_filter(&self) -> Result<EnvFilter> {
        EnvFilter::builder()
            .with_regex(self.log_filter_regex)
            .parse(&self.log)
            .map_err(|error| err!(Config("log", "{error}")))
    }

    /// The span lifecycle points to log, from `log_span_events`.
    pub fn span_events(&self) -> Result<FmtSpan> {
        fmt_span::from_str(&self.log_span_events)
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

fn true_fn() -> bool {
    true
}

fn default_log() -> String {
    // The tracing crate elides everything below ERROR in release builds, so a
    // higher level here would only cost overhead without producing output.
    cfg!(debug_assertions)
        .then_some("debug")
        .unwrap_or("info")
        .to_owned()
}

fn default_log_span_events() -> String {
    "none".to_owned()
}

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
