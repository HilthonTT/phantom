//! The database: where it lives, how much it caches, and RocksDB itself.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Database {
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

    /// Room summaries kept in the in-memory space hierarchy cache.
    ///
    /// Only summaries fetched from another server are cached — a local room is
    /// summarized from state that is already in memory, and caching it would
    /// only mean showing a stale name after a rename. Entries are small, and
    /// cache_capacity_modifier scales this along with every other cache.
    ///
    /// default: 1000
    #[serde(default = "default_space_hierarchy_cache_capacity")]
    pub space_hierarchy_cache_capacity: u32,

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
}
