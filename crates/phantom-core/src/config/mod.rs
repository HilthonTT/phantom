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

    /// Path phantom writes online database backups to. The backups are taken
    /// through RocksDB's backup engine, so the server does not have to be
    /// stopped to take one.
    ///
    /// Leave this unset to disable backups.
    ///
    /// example: "/opt/phantom-db-backups"
    pub database_backup_path: Option<PathBuf>,

    /// How many online backups to retain under `database_backup_path` before
    /// the oldest is deleted. A negative value retains every backup.
    ///
    /// default: 1
    #[serde(default = "default_database_backups_to_keep")]
    pub database_backups_to_keep: i16,

    /// Megabytes of memory the database engine is told it may use for read
    /// caches. Raising this trades memory for fewer disk reads.
    ///
    /// Like the individual caches, the default scales with the CPU core count.
    ///
    /// default: 128.0 + (64.0 * CPU core count)
    #[serde(default = "default_db_cache_capacity_mb")]
    pub db_cache_capacity_mb: f64,

    /// Megabytes of memory the database engine is told it may use for write
    /// buffers, across all columns.
    ///
    /// default: 48.0 + (4.0 * CPU core count)
    #[serde(default = "default_db_write_buffer_capacity_mb")]
    pub db_write_buffer_capacity_mb: f64,

    /// Multiplier applied to every cache capacity phantom derives from the
    /// options above. Lower it to cut memory use across the board without
    /// tuning each cache, raise it to spend more.
    ///
    /// default: 1.0
    #[serde(default = "default_cache_capacity_modifier")]
    pub cache_capacity_modifier: f64,

    /// Enables atomic flush in RocksDB. Not intended for general use: it may
    /// improve database integrity across an unclean shutdown at the cost of
    /// write throughput, and it disables pipelined writes.
    #[serde(default)]
    pub rocksdb_atomic_flush: bool,

    /// Compress the bottommost level, where the oldest and least-read data
    /// settles.
    ///
    /// Costs CPU during compaction and saves storage. Best paired with zstd.
    ///
    /// See https://github.com/facebook/rocksdb/wiki/Compression for details.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rocksdb_bottommost_compression: bool,

    /// Compression level for the bottommost level. 32767 is RocksDB's magic
    /// number for "the algorithm's own default", which differs per algorithm.
    ///
    /// Since the bottommost level is the least-read data, a level far more
    /// aggressive than the default is often worth the compaction cost. phantom
    /// substitutes its own per-column levels while this holds the default.
    ///
    /// default: 32767
    #[serde(default = "default_rocksdb_compression_level")]
    pub rocksdb_bottommost_compression_level: i32,

    /// Verify block checksums on read. Checksumming is usually hardware
    /// accelerated and cheap; older or slower platforms may gain from
    /// disabling it, at the cost of not detecting corruption on read.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rocksdb_checksums: bool,

    /// Run background compaction.
    ///
    /// You should never need to turn this off. A database that is not
    /// compacted grows without bound, reads slow down as levels pile up, and
    /// both startup and shutdown get slower.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rocksdb_compaction: bool,

    /// Run compaction threads at idle I/O priority, so compaction cannot
    /// starve request handling of disk bandwidth.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rocksdb_compaction_ioprio_idle: bool,

    /// Run compaction threads at idle CPU priority. Off by default: on a busy
    /// server it lets compaction fall arbitrarily far behind.
    #[serde(default)]
    pub rocksdb_compaction_prio_idle: bool,

    /// Compression algorithm for the database.
    ///
    /// One of "zstd", "zlib", "bz2", "lz4", "lz4hc", "snappy", or "none".
    ///
    /// zstd is the best balance of speed, storage, and CPU. lz4 spends less
    /// CPU for less compression. "none" disables compression entirely.
    ///
    /// See https://github.com/facebook/rocksdb/wiki/Compression for details.
    ///
    /// default: "zstd"
    #[serde(default = "default_rocksdb_compression_algo")]
    pub rocksdb_compression_algo: String,

    /// Compression level for `rocksdb_compression_algo`. 32767 is RocksDB's
    /// magic number for "the algorithm's own default", which differs per
    /// algorithm. phantom substitutes its own per-column levels while this
    /// holds the default.
    ///
    /// default: 32767
    #[serde(default = "default_rocksdb_compression_level")]
    pub rocksdb_compression_level: i32,

    /// Bypass the system page cache with direct I/O.
    ///
    /// Set this to false when the database lives on a filesystem that handles
    /// direct I/O poorly or not at all, such as FUSE mounts and some ZFS
    /// setups.
    ///
    /// See https://github.com/facebook/rocksdb/wiki/Direct-IO for details.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rocksdb_direct_io: bool,

    /// Log level for the database engine itself, which is separate from
    /// phantom's `log`. These messages land in `LOG` files inside the database
    /// directory. Accepts "debug", "info", "warn", "error", or "fatal".
    ///
    /// default: "error"
    #[serde(default = "default_rocksdb_log_level")]
    pub rocksdb_log_level: String,

    /// Seconds before the database engine rotates its `LOG` file regardless of
    /// size. 0 disables time-based rotation.
    ///
    /// default: 0
    #[serde(default)]
    pub rocksdb_log_time_to_roll: usize,

    /// Size in bytes a `LOG` file may reach before it is rotated.
    ///
    /// default: 4194304
    #[serde(default = "default_rocksdb_max_log_file_size")]
    pub rocksdb_max_log_file_size: usize,

    /// How many `LOG` files to keep. Must be at least 1.
    ///
    /// default: 3
    #[serde(default = "default_rocksdb_max_log_files")]
    pub rocksdb_max_log_files: usize,

    /// Tune the database for rotational storage.
    ///
    /// This skips the file-size and statistics checks that make opening a
    /// database on a hard drive slow, and stops phantom from reading ahead as
    /// aggressively as it would on an SSD.
    #[serde(default)]
    pub rocksdb_optimize_for_spinning_disks: bool,

    /// Threads the database engine may use for background work: compaction,
    /// flushes, syncs, and cleanup. 0 means one per logical CPU.
    ///
    /// default: 0
    #[serde(default)]
    pub rocksdb_parallelism_threads: usize,

    /// Run paranoid SST file checks. Catches more corruption at the cost of
    /// doing more work on every file the engine touches.
    ///
    /// See https://github.com/facebook/rocksdb/wiki/Online-Verification for
    /// details.
    #[serde(default)]
    pub rocksdb_paranoid_file_checks: bool,

    /// Open the database read-only. Writes fail; useful for inspecting a
    /// database without risking it.
    #[serde(default)]
    pub rocksdb_read_only: bool,

    /// How the engine recovers from a corrupt write-ahead log, for when the
    /// server reports corruption and refuses to start:
    ///
    /// 0 = AbsoluteConsistency — never roll anything back; fail instead.
    /// 1 = TolerateCorruptedTailRecords — drop a torn trailing record.
    /// 2 = PointInTime — resume from the last consistent point, losing the
    ///     seconds or minutes before the crash. Use this to recover.
    /// 3 = SkipAnyCorruptedRecord — skip past corruption anywhere in the log.
    ///     A last resort that can leave the database inconsistent.
    ///
    /// Mode 1 is the default: a Matrix homeserver can re-fetch what a torn
    /// tail record loses over federation. After recovering with mode 2 or 3,
    /// set this back to 1 and restart.
    ///
    /// See https://github.com/facebook/rocksdb/wiki/WAL-Recovery-Modes.
    ///
    /// default: 1
    #[serde(default = "default_rocksdb_recovery_mode")]
    pub rocksdb_recovery_mode: u8,

    /// Repair the database on startup, for when corruption is reported while
    /// running rather than at startup — errors naming SST files, typically.
    /// Try `rocksdb_recovery_mode` first if the server will not start at all.
    ///
    /// Back the database directory up before repairing, and turn this back off
    /// once the repair has succeeded.
    #[serde(default)]
    pub rocksdb_repair: bool,

    /// Open the database as a secondary instance of a primary held by another
    /// process. The instance is read-only and catches up on demand.
    #[serde(default)]
    pub rocksdb_secondary: bool,

    /// How much statistics the engine collects, from 0 to 6. Some admin
    /// commands need this above 0; higher levels cost performance.
    ///
    /// 0 = none.
    /// 1 = none in release builds, all but the detailed timers in debug ones.
    /// 2 to 3 = statistics with no measurable impact.
    /// 4 to 5 = statistics with a possible impact.
    /// 6 = everything.
    ///
    /// default: 1
    #[serde(default = "default_rocksdb_stats_level")]
    pub rocksdb_stats_level: u8,

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

fn default_database_backups_to_keep() -> i16 {
    1
}

fn default_db_cache_capacity_mb() -> f64 {
    128.0 + parallelism_scaled_f64(64.0)
}

fn default_db_write_buffer_capacity_mb() -> f64 {
    48.0 + parallelism_scaled_f64(4.0)
}

fn default_cache_capacity_modifier() -> f64 {
    1.0
}

/// RocksDB reads 32767 as "use whatever this algorithm calls its default
/// level", since the range of valid levels differs per algorithm. It is the
/// sentinel phantom watches for before substituting a per-column level of its
/// own.
fn default_rocksdb_compression_level() -> i32 {
    32767
}

fn default_rocksdb_compression_algo() -> String {
    "zstd".to_owned()
}

fn default_rocksdb_log_level() -> String {
    "error".to_owned()
}

fn default_rocksdb_max_log_file_size() -> usize {
    4 * 1024 * 1024
}

fn default_rocksdb_max_log_files() -> usize {
    3
}

fn default_rocksdb_recovery_mode() -> u8 {
    1
}

fn default_rocksdb_stats_level() -> u8 {
    1
}

/// Scales a per-core figure by the parallelism actually available to this
/// process, which is what the memory defaults above are expressed in.
fn parallelism_scaled_f64(val: f64) -> f64 {
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    let cores = crate::utils::available_parallelism() as f64;

    val * cores
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
