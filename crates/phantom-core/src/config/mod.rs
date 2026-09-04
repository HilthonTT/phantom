//! Server configuration.
//!
//! Every field below is a config option: its doc comment is the documentation
//! users read, and `#[config_example_generator]` turns this struct into
//! `phantom-example.toml` at the workspace root on every `cargo build`. Edit
//! the docs here, never that file — it is regenerated and overwritten.

mod defaults;
mod listen;
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

use self::{
    defaults::*,
    listen::{ListeningAddr, ListeningPort},
};
pub use self::{
    listen::IpLookupStrategy, manager::Manager, proxy::ProxyConfig, validate::validate,
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

    /// Entries kept in the in-memory state-info cache.
    ///
    /// One entry is the stack of compressed-state layers behind a single
    /// shortstatehash, so an entry is far larger than an auth chain's and the
    /// default is correspondingly smaller. Every read of a room's state goes
    /// through it, so a miss costs a walk down the diff layers to the full
    /// state at the bottom. cache_capacity_modifier scales it along with every
    /// other cache.
    ///
    /// default: 100 + (10 * CPU core count)
    #[serde(default = "default_stateinfo_cache_capacity")]
    pub stateinfo_cache_capacity: u32,

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

    /// Password set on the server's own user account so an operator locked out
    /// of every admin account can log in as it and recover one.
    ///
    /// While this is set the server user is a usable account with the default
    /// push ruleset. Unset it once recovery is done: clearing it deactivates
    /// the account again and logs out every session that was opened with it.
    ///
    /// display: sensitive
    pub emergency_password: Option<String>,

    /// Makes leaving a room also forget it, rather than leaving it in the
    /// user's `leave` section of sync until they forget it themselves.
    ///
    /// Banned and admin-disabled rooms are forgotten on leave either way.
    ///
    /// default: false
    #[serde(default)]
    pub forget_forced_upon_leave: bool,

    /// Seconds an OpenID token stays valid for.
    ///
    /// The token proves to an integration that the bearer holds the account it
    /// names, so it wants to be long enough to be exchanged and no longer.
    ///
    /// default: 3600
    #[serde(default = "default_openid_token_ttl")]
    pub openid_token_ttl: u64,

    /// Milliseconds a login token stays valid for.
    ///
    /// This is the `m.login.token` handed out to complete a login started
    /// elsewhere, so it is spent within seconds of being issued; the spec caps
    /// it at five minutes.
    ///
    /// default: 120000
    #[serde(default = "default_login_token_ttl")]
    pub login_token_ttl: u64,

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

    /// Seconds a user may be idle before their presence is moved from
    /// "online" to "unavailable".
    ///
    /// The clock starts at the last presence update the user's client sent,
    /// so a client that pings while a person is away keeps them online.
    ///
    /// default: 300
    #[serde(default = "default_presence_idle_timeout_s")]
    pub presence_idle_timeout_s: u64,

    /// Seconds a user may stay "unavailable" before their presence is moved
    /// to "offline".
    ///
    /// Measured from the same last update as `presence_idle_timeout_s`
    /// rather than from the move to "unavailable", so it wants to be the
    /// larger of the two.
    ///
    /// default: 1800
    #[serde(default = "default_presence_offline_timeout_s")]
    pub presence_offline_timeout_s: u64,

    /// Time out remote users' presence as well as local users'.
    ///
    /// A remote server sends presence for its own users and stops sending
    /// when they go quiet, which leaves them showing as online here forever.
    /// Timing them out locally is what clears that, at the cost of a timer
    /// per remote user this server has heard about.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub presence_timeout_remote_users: bool,

    /// Send local users' presence to the other servers in their rooms.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_outgoing_presence: bool,

    /// Send local users' read receipts to the other servers in their rooms.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_outgoing_read_receipts: bool,

    /// Notary servers to gather other servers' public keys from, when this
    /// server does not already hold a key it needs.
    ///
    /// example: ["matrix.org", "tchncs.de"]
    ///
    /// default: ["matrix.org"]
    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,

    /// Ask the notaries in `trusted_servers` for a key before asking the
    /// server the key belongs to.
    ///
    /// Asking the origin first is the safer order: a notary that has been
    /// compromised can only answer for keys it was asked about, and it is
    /// only asked once the origin has failed to answer. Asking the notaries
    /// first is faster, since one notary can answer for many servers at once.
    #[serde(default)]
    pub query_trusted_key_servers_first: bool,

    /// Ask the notaries first, but only while joining a room.
    ///
    /// A join gathers keys from every server in the room, which is where the
    /// per-origin round trips are most noticeable; this bounds the exposure
    /// to a compromised notary to that one operation. Ignored where
    /// `query_trusted_key_servers_first` is already on.
    #[serde(default = "true_fn")]
    pub query_trusted_key_servers_first_on_join: bool,

    /// Only ever ask the notaries in `trusted_servers` for keys, and never
    /// the server a key belongs to.
    ///
    /// For a cluster behind a notary it operates itself. With no reachable
    /// notary holding a key, that key is simply never acquired.
    #[serde(default)]
    pub only_query_trusted_key_servers: bool,

    /// Servers to ask a notary about in one batched request.
    ///
    /// default: 256
    #[serde(default = "default_trusted_server_batch_size")]
    pub trusted_server_batch_size: usize,

    /// Send federation requests to other servers.
    ///
    /// With this off the server still answers what arrives, but never
    /// initiates a request of its own, which includes fetching the signing
    /// keys needed to verify a remote event.
    #[serde(default = "true_fn")]
    pub allow_federation: bool,

    /// Servers this server refuses to send federation requests to, as regular
    /// expressions matched against the server name.
    ///
    /// A plain word is a valid pattern, and matches anywhere in the name.
    ///
    /// example: ["badserver\\.tld$", "badphrase", "19dollarfortnitecards"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_server_names: RegexSet,

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

    /// Longest a remote server that keeps failing is left alone between
    /// attempts. The wait after a failure doubles each time, starting from
    /// `sender_timeout`, and stops growing here.
    ///
    /// default: 86400
    #[serde(default = "default_sender_retry_backoff_limit")]
    pub sender_retry_backoff_limit: u64,

    /// Seconds the sender waits for its in-flight transactions to finish at
    /// shutdown before giving up on them. Nothing is lost either way: what
    /// was not acknowledged is sent again at the next start.
    ///
    /// default: 5
    #[serde(default = "default_sender_shutdown_timeout")]
    pub sender_shutdown_timeout: u64,

    /// Worker tasks the sender spreads its destinations over. Each remote
    /// server always lands on the same worker, so ordering to one server is
    /// kept. Never more than the runtime has threads.
    ///
    /// 0 runs a single worker.
    ///
    /// default: 0
    #[serde(default)]
    pub sender_workers: usize,

    /// Send the transactions that were still in flight when the server last
    /// stopped as soon as it starts. Off, they wait until there is something
    /// new to send to that server.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub startup_netburst: bool,

    /// Events per remote server the startup burst may carry; the rest are
    /// dropped. Servers that were unreachable for a long time can have a lot
    /// queued, and sending all of it at once is rarely wanted. -1 keeps
    /// everything.
    ///
    /// default: 50
    #[serde(default = "default_startup_netburst_keep")]
    pub startup_netburst_keep: i64,

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

    /// Admin commands to run once the server has started, in order, as if
    /// they had been typed into the admin room.
    ///
    /// Each entry is one command without its `!admin` prefix. Their output
    /// goes to the log, since there is nobody in a room to answer, and a
    /// command that fails stops startup unless
    /// `admin_execute_errors_ignore` is set.
    ///
    /// This build registers no command set, so anything listed here fails.
    /// The option is here because the schedule belongs to the admin service
    /// and the commands do not.
    ///
    /// example: ["users create-user @admin:example.com", "server memory-usage"]
    ///
    /// default: []
    #[serde(default)]
    pub admin_execute: Vec<String>,

    /// Admin commands to run every time the server is sent SIGUSR2, in the
    /// same form as `admin_execute`.
    ///
    /// Unlike the startup list this one is re-read each time, so a reloaded
    /// config changes what the next signal runs.
    ///
    /// default: []
    #[serde(default)]
    pub admin_signal_execute: Vec<String>,

    /// Carry on when one of the commands above fails, instead of treating the
    /// failure as fatal to startup.
    ///
    /// default: false
    #[serde(default)]
    pub admin_execute_errors_ignore: bool,

    /// Let an admin run a command outside the admin room by escaping it with a
    /// backslash, as `\!admin ...`.
    ///
    /// The command and its output are both visible to that room, which is the
    /// point: it is how an admin answers a question where it was asked. Only
    /// local admins can do it, escaped or not.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub admin_escape_commands: bool,

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

/// Config options that older versions of phantom accepted. They are still
/// parsed into `catchall` so that `validate` can name them, rather than being
/// reported as unknown.
const DEPRECATED_KEYS: &[&str] = &[];

#[cfg(test)]
mod tests;
