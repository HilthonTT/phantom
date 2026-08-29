//! Server configuration.
//!
//! Every field below is a config option: its doc comment is the documentation
//! users read, and `#[config_example_generator]` turns this struct into
//! `phantom-example.toml` at the workspace root on every `cargo build`. Edit
//! the docs here, never that file — it is regenerated and overwritten.

pub mod manager;
pub mod proxy;
pub mod validate;

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
use regex::RegexSet;
use ruma::OwnedServerName;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, fmt::format::FmtSpan};

pub use self::{manager::Manager, proxy::ProxyConfig, validate::validate};
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

    /// Entries kept in the in-memory auth chain cache.
    ///
    /// An auth chain is the set of events authorizing one event, and
    /// recomputing it walks the room's state; caching them is what keeps state
    /// resolution off the database. Entries are small, so the default is
    /// generous, and cache_capacity_modifier scales it along with every other
    /// cache.
    ///
    /// default: 100000 + (10000 * CPU core count)
    #[serde(default = "default_auth_chain_cache_capacity")]
    pub auth_chain_cache_capacity: u32,

    /// Number of database read workers to spawn per hardware queue, where
    /// phantom could not learn the queue's own depth from the operating
    /// system.
    ///
    /// These are operating-system threads, not tokio tasks: a read that misses
    /// the block cache blocks until the storage answers, and doing that on a
    /// tokio worker would stall every other task sharing it.
    ///
    /// default: 32
    #[serde(default = "default_db_pool_workers")]
    pub db_pool_workers: usize,

    /// Ceiling on the workers derived for one hardware queue, per CPU core
    /// that queue serves.
    ///
    /// Only bites where the storage reports a queue depth far above what the
    /// cores feeding it could keep busy.
    ///
    /// default: 64
    #[serde(default = "default_db_pool_workers_limit")]
    pub db_pool_workers_limit: usize,

    /// Requests a queue accepts per worker servicing it, before submitting
    /// blocks.
    ///
    /// The queue is the handoff between the tokio workers producing requests
    /// and the pool workers draining them. Backpressure here is deliberate: it
    /// is what stops a burst of requests from being read off the network
    /// faster than the storage can answer them.
    ///
    /// default: 4
    #[serde(default = "default_db_pool_queue_mult")]
    pub db_pool_queue_mult: usize,

    /// Pin each pool worker to the cores its hardware queue is served by.
    ///
    /// Keeps a request, its worker, and the queue that will carry it to the
    /// device on the same node. Has no effect where there is one queue.
    #[serde(default = "true_fn")]
    pub db_pool_affinity: bool,

    /// Scales the concurrency the stream combinators run at, which phantom
    /// derives from the pool topology once the database is open.
    ///
    /// Zero leaves the built-in defaults alone.
    ///
    /// default: 1.0
    #[serde(default = "default_stream_width_scale")]
    pub stream_width_scale: f32,

    /// Requests a stream gathers before handing a batch to the database.
    ///
    /// Batching is what lets one queue submission cover many keys; the cost is
    /// latency for the first key in a batch.
    ///
    /// default: 1024
    #[serde(default = "default_stream_amplification")]
    pub stream_amplification: usize,

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

    /// Path to a file holding the registration token instead of writing it
    /// into the config. The contents are read once at startup, with
    /// surrounding whitespace trimmed, and take priority over
    /// `registration_token`.
    ///
    /// example: "/etc/phantom/.reg_token"
    pub registration_token_file: Option<PathBuf>,

    /// Text appended to a user's displayname when they register, after a
    /// space. Leave it empty to append nothing.
    ///
    /// example: "🏳️‍⚧️"
    ///
    /// default: ""
    #[serde(default)]
    pub new_user_displayname_suffix: String,

    /// Allow ordinary users to create rooms. Admins and appservices may
    /// always create them regardless of this.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_room_creation: bool,

    /// Serve this server's public room directory to other servers over
    /// federation.
    ///
    /// Leaving this off keeps the directory from being crawled by remote
    /// spiders, at the cost of your rooms not appearing in other servers'
    /// directory searches.
    #[serde(default)]
    pub allow_public_room_directory_over_federation: bool,

    /// Send device display names to other servers, so remote users see what a
    /// local user named their session.
    ///
    /// Off by default: the names are frequently identifying, and nothing in
    /// the protocol needs them.
    #[serde(default)]
    pub allow_device_name_federation: bool,

    /// Notary servers to gather other servers' public keys from, when this
    /// server does not already hold a key it needs.
    ///
    /// example: ["matrix.org", "tchncs.de"]
    ///
    /// default: ["matrix.org"]
    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,

    /// Periodically fetch phantom's announcement feed, which carries security
    /// and release notices. Despite the name this checks for announcements,
    /// not for a newer version to install.
    #[serde(default)]
    pub allow_check_for_updates: bool,

    /// Static TURN username handed to clients, for a TURN server that
    /// authenticates with fixed credentials rather than `turn_secret`.
    ///
    /// default: ""
    #[serde(default)]
    pub turn_username: String,

    /// Static TURN password handed to clients. See `turn_username`.
    ///
    /// display: sensitive
    /// default: ""
    #[serde(default)]
    pub turn_password: String,

    /// TURN servers to hand to clients, as URIs. Use the `turns:` scheme
    /// rather than `turn:` for TURN over TLS.
    ///
    /// example: ["turn:example.turn.uri?transport=udp",
    /// "turn:example.turn.uri?transport=tcp"]
    ///
    /// default: []
    #[serde(default)]
    pub turn_uris: Vec<String>,

    /// Shared secret the TURN server is configured with, from which phantom
    /// derives the time-limited credentials it hands each client.
    ///
    /// Preferred over the static `turn_username`/`turn_password` pair, since
    /// a credential phantom derives expires on its own.
    ///
    /// display: sensitive
    /// default: ""
    #[serde(default)]
    pub turn_secret: String,

    /// Path to a file holding the TURN shared secret instead of writing it
    /// into the config. The contents are read once at startup, with
    /// surrounding whitespace trimmed, and take priority over `turn_secret`;
    /// a file that cannot be read falls back to it.
    ///
    /// example: "/etc/phantom/.turn_secret"
    pub turn_secret_file: Option<PathBuf>,

    /// How long, in seconds, a TURN credential phantom derives stays valid.
    ///
    /// default: 86400
    #[serde(default = "default_turn_ttl")]
    pub turn_ttl: u64,

    /// Path a push gateway is expected to serve its notify endpoint at. Only
    /// the appservice-style pushers that do not carry their own URL use it.
    ///
    /// default: "/_matrix/push/v1/notify"
    #[serde(default = "default_notification_push_path")]
    pub notification_push_path: String,

    /// Domains phantom may fetch URL previews from, matched as a substring of
    /// the URL's host.
    ///
    /// "google.com" matches `https://google.com` and also
    /// `http://notgoogle.com.example`, so prefer
    /// `url_preview_domain_explicit_allowlist` where you can. "*" allows
    /// every domain, which lets any user aim this server at any host on its
    /// network.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_contains_allowlist: Vec<String>,

    /// Domains phantom may fetch URL previews from, matched exactly.
    ///
    /// "google.com" matches `https://google.com` but not
    /// `https://notgoogle.com.example`. See
    /// `url_preview_check_root_domain` for matching subdomains too.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_explicit_allowlist: Vec<String>,

    /// Domains phantom may never fetch URL previews from, matched exactly.
    /// Checked before either allowlist, so it always wins.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_domain_explicit_denylist: Vec<String>,

    /// URLs phantom may fetch previews from, matched as a substring of the
    /// whole URL rather than of its host.
    ///
    /// This matches anywhere in the URL, so "google.com" also matches
    /// `https://example.invalid/google.com`. "*" allows every URL.
    ///
    /// default: []
    #[serde(default)]
    pub url_preview_url_contains_allowlist: Vec<String>,

    /// Bytes of a page phantom reads before giving up on finding its preview
    /// metadata.
    ///
    /// default: 256000
    #[serde(default = "default_url_preview_max_spider_size")]
    pub url_preview_max_spider_size: usize,

    /// Apply the domain allowlists to a URL's root domain rather than to the
    /// host it names, so that allowing "wikipedia.org" also allows
    /// "en.m.wikipedia.org". Does not affect
    /// `url_preview_url_contains_allowlist`.
    #[serde(default)]
    pub url_preview_check_root_domain: bool,

    /// Address, or the name of a network interface, that URL preview requests
    /// are sent from. Unset lets the operating system pick.
    ///
    /// Interface names work on Linux, Android and Fuchsia; elsewhere only an
    /// address is accepted, and a name is rejected at startup.
    ///
    /// example: "eth0" or "1.2.3.4"
    ///
    /// default:
    #[serde(default, with = "either::serde_untagged_optional")]
    pub url_preview_bound_interface: Option<Either<IpAddr, String>>,

    /// CIDR ranges phantom will not send an outbound request to, which is
    /// what keeps a URL preview or a media fetch from being aimed at the
    /// network this server is on.
    ///
    /// This is enforced in phantom, so it is a backstop rather than a
    /// boundary: a firewall is what actually contains the server. Requests
    /// through a proxy are not checked against it. Set it to `[]` to disable.
    ///
    /// The default covers the loopback, private, link-local, shared-address,
    /// documentation, benchmarking, multicast and unique-local ranges of both
    /// address families.
    ///
    /// default: ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12",
    /// "192.168.0.0/16", "100.64.0.0/10", "192.0.0.0/24", "169.254.0.0/16",
    /// "192.88.99.0/24", "198.18.0.0/15", "192.0.2.0/24", "198.51.100.0/24",
    /// "203.0.113.0/24", "224.0.0.0/4", "::1/128", "fe80::/10", "fc00::/7",
    /// "2001:db8::/32", "ff00::/8", "fec0::/10"]
    #[serde(default = "default_ip_range_denylist")]
    pub ip_range_denylist: Vec<String>,

    /// Proxy the outbound requests go through.
    ///
    /// `"none"` sends them directly. A table under `[global.proxy]` either
    /// proxies everything:
    ///
    /// ```toml
    /// [global.proxy]
    /// global = { url = "socks5h://localhost:9050" }
    /// ```
    ///
    /// or proxies the domains that match a rule, which is what reaching
    /// `.onion` servers over Tor while everything else goes direct looks
    /// like:
    ///
    /// ```toml
    /// [[global.proxy.by_domain]]
    /// url = "socks5h://localhost:9050"
    /// include = ["*.onion", "matrix.myspecial.onion"]
    /// exclude = ["*.myspecial.onion"]
    /// ```
    ///
    /// An empty `include` reads as `["*"]`. Where a domain matches both
    /// lists, the more specific rule decides — above, the proxy is used for
    /// `ordinary.onion` and `matrix.myspecial.onion`, but not for
    /// `hello.myspecial.onion`.
    ///
    /// Note that `ip_range_denylist` is not applied to a proxied request.
    ///
    /// default: "none"
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Seconds phantom waits to establish a connection, for the requests that
    /// have no timeout of their own: URL previews and the announcement check.
    ///
    /// default: 10
    #[serde(default = "default_request_conn_timeout")]
    pub request_conn_timeout: u64,

    /// Seconds phantom waits for more data on one of those connections before
    /// giving up on the response.
    ///
    /// default: 35
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// Seconds one of those requests may take in total. Deliberately far
    /// above `request_timeout`: this is the backstop that stops a request
    /// from living forever, not the one that should normally fire.
    ///
    /// default: 320
    #[serde(default = "default_request_total_timeout")]
    pub request_total_timeout: u64,

    /// Seconds an unused connection is kept in the pool for those requests.
    ///
    /// default: 5
    #[serde(default = "default_request_idle_timeout")]
    pub request_idle_timeout: u64,

    /// Unused connections kept per host for those requests. One is usually
    /// right: the next request to a host can reuse the connection the last
    /// one left.
    ///
    /// default: 1
    #[serde(default = "default_request_idle_per_host")]
    pub request_idle_per_host: u16,

    /// Seconds phantom waits to connect while resolving another server's
    /// `.well-known/matrix/server`.
    ///
    /// default: 6
    #[serde(default = "default_well_known_conn_timeout")]
    pub well_known_conn_timeout: u64,

    /// Seconds a `.well-known` request may take in total.
    ///
    /// default: 10
    #[serde(default = "default_well_known_timeout")]
    pub well_known_timeout: u64,

    /// Seconds phantom waits for more data from another homeserver.
    ///
    /// Worth keeping high: a join to a large room is one request, and the
    /// remote server may be some time assembling the response.
    ///
    /// default: 300
    #[serde(default = "default_federation_timeout")]
    pub federation_timeout: u64,

    /// Seconds an unused federation connection is kept in the pool.
    ///
    /// default: 25
    #[serde(default = "default_federation_idle_timeout")]
    pub federation_idle_timeout: u64,

    /// Unused federation connections kept per remote server.
    ///
    /// default: 1
    #[serde(default = "default_federation_idle_per_host")]
    pub federation_idle_per_host: u16,

    /// Seconds a transaction the sender pushes to another server may take.
    /// The remote server has to process everything in it before answering.
    ///
    /// default: 180
    #[serde(default = "default_sender_timeout")]
    pub sender_timeout: u64,

    /// Seconds an unused sender connection is kept in the pool.
    ///
    /// default: 180
    #[serde(default = "default_sender_idle_timeout")]
    pub sender_idle_timeout: u64,

    /// Seconds a request to an appservice may take. Appservices usually sit
    /// on the same network, so this is about the work they do rather than the
    /// distance.
    ///
    /// default: 35
    #[serde(default = "default_appservice_timeout")]
    pub appservice_timeout: u64,

    /// Seconds an unused appservice connection is kept in the pool.
    ///
    /// default: 300
    #[serde(default = "default_appservice_idle_timeout")]
    pub appservice_idle_timeout: u64,

    /// Seconds an unused push gateway connection is kept in the pool.
    ///
    /// default: 15
    #[serde(default = "default_pusher_idle_timeout")]
    pub pusher_idle_timeout: u64,

    /// Accept and decompress gzip-encoded responses.
    ///
    /// Compression on a TLS connection can leak plaintext to someone watching
    /// the sizes; see https://en.wikipedia.org/wiki/BREACH. Off unless the
    /// bandwidth matters more.
    #[serde(default)]
    pub gzip_compression: bool,

    /// Accept and decompress brotli-encoded responses. See
    /// `gzip_compression`.
    #[serde(default)]
    pub brotli_compression: bool,

    /// Accept and decompress zstd-encoded responses. See `gzip_compression`.
    #[serde(default)]
    pub zstd_compression: bool,

    /// Send federation requests to this server itself, which nothing but a
    /// bug or a development setup has a reason to do.
    #[serde(default)]
    pub federation_loopback: bool,

    /// Skip TLS certificate validation on every outbound request.
    ///
    /// There is no safe use of this outside a lab: it hands anyone who can
    /// intercept the connection everything that goes over it, federation
    /// traffic included. `validate` refuses to let it pass quietly.
    #[serde(default)]
    pub allow_invalid_tls_certificates: bool,

    /// Entries the in-process DNS cache holds. Entries vary in size, so raise
    /// this carefully. Lower it only where a real external cache sits in
    /// front of phantom — systemd-resolved is not one.
    ///
    /// default: 32768
    #[serde(default = "default_dns_cache_entries")]
    pub dns_cache_entries: u32,

    /// Seconds a resolved name is held even if its record says less.
    ///
    /// default: 10800
    #[serde(default = "default_dns_min_ttl")]
    pub dns_min_ttl: u64,

    /// Seconds an NXDOMAIN is held. Three days by default, and deliberately:
    /// a name that does not resolve is almost never about to, and rechecking
    /// each one is what makes a server with dead remotes in its rooms slow.
    ///
    /// default: 259200
    #[serde(default = "default_dns_min_ttl_nxdomain")]
    pub dns_min_ttl_nxdomain: u64,

    /// Attempts made at a nameserver before the query is given up on.
    ///
    /// default: 10
    #[serde(default = "default_dns_attempts")]
    pub dns_attempts: u16,

    /// Seconds phantom waits for a nameserver to answer. Recursive queries
    /// take seconds on some domains, so a low value here reads as a DNS
    /// outage.
    ///
    /// default: 10
    #[serde(default = "default_dns_timeout")]
    pub dns_timeout: u64,

    /// Retry a query over TCP when the UDP one errors or is truncated.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub dns_tcp_fallback: bool,

    /// Query the configured nameservers over TCP only. Some container
    /// networks need this, where UDP replies are truncated or dropped.
    #[serde(default)]
    pub query_over_tcp_only: bool,

    /// Keep asking the remaining nameservers after one says the name does not
    /// exist, rather than trusting the first negative answer.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub query_all_nameservers: bool,

    /// Which address records to look up, and in what order.
    ///
    /// One of "ipv4-only", "ipv6-only", "ipv4-and-ipv6" (both at once, first
    /// answer wins), "ipv6-then-ipv4", or "ipv4-then-ipv6".
    ///
    /// On a host with no IPv6 route, "ipv4-only" saves a lookup whose answer
    /// could never be connected to.
    ///
    /// default: "ipv4-then-ipv6"
    #[serde(default)]
    pub ip_lookup_strategy: IpLookupStrategy,

    /// Room aliases and room IDs that may not be created, as regular
    /// expressions. A plain word is a valid pattern, and matches anywhere in
    /// the alias.
    ///
    /// Checked when an alias or a custom room ID is created, and at startup
    /// against the aliases already in the database, which are reported as
    /// warnings rather than removed.
    ///
    /// example: ["19dollarfortnitecards", "b[4a]droom", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_alias_names: RegexSet,

    /// Usernames that may not be registered, as regular expressions. A plain
    /// word is a valid pattern, and matches anywhere in the username.
    ///
    /// Checked on the availability request and on registration, and at
    /// startup against the users already in the database, which are reported
    /// as warnings rather than removed.
    ///
    /// example: ["administrator", "b[a4]dusernam[3e]", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_usernames: RegexSet,

    /// Reload the configuration when the server is sent SIGUSR1.
    ///
    /// Only `server_name` is fixed for the life of the process; every other
    /// option is re-read. Has no effect where the platform has no SIGUSR1.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub config_reload_signal: bool,

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

        validate(&config)?;

        Ok(config)
    }

    /// The console layer's filter, built from `log` and `log_filter_regex`.
    ///
    /// Lives here rather than at the logging callsite so that [`validate`] can
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

/// Which address records [`Config::ip_lookup_strategy`] asks for.
///
/// The reference spells this as a number 1 through 5, which nothing but its
/// own documentation can decode; the names the resolver already uses are
/// spelled out here instead.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IpLookupStrategy {
    /// A records only.
    Ipv4Only,

    /// AAAA records only.
    Ipv6Only,

    /// Both at once; whichever answers first is used.
    Ipv4AndIpv6,

    /// AAAA, falling back to A.
    Ipv6ThenIpv4,

    /// A, falling back to AAAA.
    #[default]
    Ipv4ThenIpv6,
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
/// parsed into `catchall` so that `validate` can name them, rather than being
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

fn default_auth_chain_cache_capacity() -> u32 {
    parallelism_scaled_u32(10_000).saturating_add(100_000)
}

fn default_db_pool_workers() -> usize {
    32
}

fn default_db_pool_workers_limit() -> usize {
    64
}

fn default_db_pool_queue_mult() -> usize {
    4
}

fn default_stream_width_scale() -> f32 {
    1.0
}

fn default_stream_amplification() -> usize {
    1024
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

fn default_trusted_servers() -> Vec<OwnedServerName> {
    vec![OwnedServerName::try_from("matrix.org").expect("matrix.org is a valid server name")]
}

fn default_turn_ttl() -> u64 {
    60 * 60 * 24
}

fn default_notification_push_path() -> String {
    "/_matrix/push/v1/notify".to_owned()
}

fn default_url_preview_max_spider_size() -> usize {
    256_000
}

/// Every range that has no business being reached from the public internet:
/// loopback, the three private ranges, shared address space, IETF protocol
/// assignments, link-local, 6to4 relay anycast, benchmarking, the three
/// documentation ranges, and multicast — then the v6 equivalents.
fn default_ip_range_denylist() -> Vec<String> {
    [
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "192.0.0.0/24",
        "169.254.0.0/16",
        "192.88.99.0/24",
        "198.18.0.0/15",
        "192.0.2.0/24",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "224.0.0.0/4",
        "::1/128",
        "fe80::/10",
        "fc00::/7",
        "2001:db8::/32",
        "ff00::/8",
        "fec0::/10",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

fn default_request_conn_timeout() -> u64 {
    10
}

fn default_request_timeout() -> u64 {
    35
}

fn default_request_total_timeout() -> u64 {
    320
}

fn default_request_idle_timeout() -> u64 {
    5
}

fn default_request_idle_per_host() -> u16 {
    1
}

fn default_well_known_conn_timeout() -> u64 {
    6
}

fn default_well_known_timeout() -> u64 {
    10
}

fn default_federation_timeout() -> u64 {
    300
}

fn default_federation_idle_timeout() -> u64 {
    25
}

fn default_federation_idle_per_host() -> u16 {
    1
}

fn default_sender_timeout() -> u64 {
    180
}

fn default_sender_idle_timeout() -> u64 {
    180
}

fn default_appservice_timeout() -> u64 {
    35
}

fn default_appservice_idle_timeout() -> u64 {
    300
}

fn default_pusher_idle_timeout() -> u64 {
    15
}

fn default_dns_cache_entries() -> u32 {
    32768
}

fn default_dns_min_ttl() -> u64 {
    60 * 60 * 3
}

fn default_dns_min_ttl_nxdomain() -> u64 {
    60 * 60 * 24 * 3
}

fn default_dns_attempts() -> u16 {
    10
}

fn default_dns_timeout() -> u64 {
    10
}

/// Scales a per-core figure by the parallelism actually available to this
/// process, which is what the memory defaults above are expressed in.
fn parallelism_scaled_f64(val: f64) -> f64 {
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    let cores = crate::sys::compute::available_parallelism() as f64;

    val * cores
}

/// [`parallelism_scaled_f64`] for the cache capacities, which are counts of
/// entries rather than megabytes.
fn parallelism_scaled_u32(val: u32) -> u32 {
    let cores = crate::sys::compute::available_parallelism();

    usize::try_from(val)
        .map(|val| val.saturating_mul(cores))
        .map_or(u32::MAX, |val| u32::try_from(val).unwrap_or(u32::MAX))
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
            turn_secret = "swordfish"
            "#,
        )
        .expect("config is valid");

        let rendered = config.to_string();
        assert!(rendered.contains("| server_name | \"phantom.chat\" |"));
        assert!(rendered.contains("| registration_token | *********** |"));
        assert!(rendered.contains("| turn_secret | *********** |"));
        assert!(!rendered.contains("hunter2"), "secret must not be rendered");
        assert!(
            !rendered.contains("swordfish"),
            "secret must not be rendered"
        );
        assert!(
            !rendered.contains("catchall"),
            "ignored field is not a config option"
        );
    }

    #[test]
    fn regex_options_are_compiled_while_the_config_loads() {
        let config = config(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            forbidden_usernames = ["b[a4]dusernam[3e]", "badphrase"]
            "#,
        )
        .expect("config is valid");

        assert!(config.forbidden_usernames.is_match("b4dusername"));
        assert!(!config.forbidden_usernames.is_match("goodusername"));
        assert!(
            config.forbidden_alias_names.is_empty(),
            "an unset regex option is an empty set, not a set matching everything"
        );
    }

    #[test]
    fn a_malformed_regex_is_an_error() {
        assert!(
            config(
                r#"
                [global]
                server_name = "phantom.chat"
                database_path = "/var/lib/phantom"
                forbidden_usernames = ["b[adusername"]
                "#,
            )
            .is_err()
        );
    }

    #[test]
    fn an_empty_registration_token_is_an_error() {
        assert!(
            config(
                r#"
                [global]
                server_name = "phantom.chat"
                database_path = "/var/lib/phantom"
                registration_token = ""
                "#,
            )
            .is_err(),
            "an empty token is a half-written config, not a token"
        );
    }

    #[test]
    fn an_unreadable_registration_token_file_is_an_error() {
        assert!(
            config(
                r#"
                [global]
                server_name = "phantom.chat"
                database_path = "/var/lib/phantom"
                registration_token_file = "/nonexistent/phantom/.reg_token"
                "#,
            )
            .is_err(),
            "the service would silently fall back to no token at all"
        );
    }

    #[test]
    fn missing_required_option_is_an_error() {
        assert!(config("[global]\nserver_name = \"phantom.chat\"\n").is_err());
    }
}
