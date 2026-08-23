//! Database-wide options.

use std::cmp;

use phantom_core::{Config, Result, utils};
use rocksdb::{Cache, DBRecoveryMode, Env, LogLevel, Options, statistics::StatsLevel};

use super::cf_opts::cache_size_f64;

/// Options for opening the database as a whole.
///
/// These also stand in as the default column options, so every column is
/// opened by passing this result through
/// [`cf_options`](super::cf_opts::cf_options) first.
pub(super) fn db_options(config: &Config, env: &Env, row_cache: &Cache) -> Result<Options> {
    const DEFAULT_STATS_LEVEL: StatsLevel = if cfg!(debug_assertions) {
        StatsLevel::ExceptDetailedTimers
    } else {
        StatsLevel::DisableAll
    };

    let mut opts = Options::default();

    // Logging
    set_logging_defaults(&mut opts, config);

    // Processing
    opts.set_max_background_jobs(num_threads::<i32>(config)?);
    opts.set_max_subcompactions(num_threads::<u32>(config)?);
    opts.set_avoid_unnecessary_blocking_io(true);
    opts.set_max_file_opening_threads(0);

    // IO
    opts.set_manual_wal_flush(true);
    opts.set_atomic_flush(config.rocksdb_atomic_flush);
    opts.set_enable_pipelined_write(!config.rocksdb_atomic_flush);
    if config.rocksdb_direct_io {
        opts.set_use_direct_reads(true);
        opts.set_use_direct_io_for_flush_and_compaction(true);
    }
    if config.rocksdb_optimize_for_spinning_disks {
        // Speeds up opening the database on a hard drive, where gathering the
        // statistics costs a seek per file.
        opts.set_skip_stats_update_on_db_open(true);
    } else {
        opts.set_compaction_readahead_size(1024 * 512);
    }

    // Blocks
    opts.set_row_cache(row_cache);
    opts.set_db_write_buffer_size(cache_size_f64(
        config,
        config.db_write_buffer_capacity_mb,
        1_048_576,
    )?);

    // Files
    opts.set_table_cache_num_shard_bits(7);
    opts.set_wal_size_limit_mb(1024);
    opts.set_max_total_wal_size(1024 * 1024 * 512);
    opts.set_writable_file_max_buffer_size(1024 * 1024 * 2);

    // Misc
    opts.set_disable_auto_compactions(!config.rocksdb_compaction);
    opts.create_missing_column_families(true);
    opts.create_if_missing(true);

    opts.set_statistics_level(match config.rocksdb_stats_level {
        0 => StatsLevel::DisableAll,
        1 => DEFAULT_STATS_LEVEL,
        2 => StatsLevel::ExceptHistogramOrTimers,
        3 => StatsLevel::ExceptTimers,
        4 => StatsLevel::ExceptDetailedTimers,
        5 => StatsLevel::ExceptTimeForMutex,
        6_u8..=u8::MAX => StatsLevel::All,
    });

    opts.set_report_bg_io_stats(match config.rocksdb_stats_level {
        0..=1 => false,
        2_u8..=u8::MAX => true,
    });

    // An unclean shutdown of a homeserver is usually survivable: whatever a
    // torn tail record loses can be re-fetched over federation. `config::check`
    // has already rejected any mode outside this range.
    opts.set_wal_recovery_mode(match config.rocksdb_recovery_mode {
        0 => DBRecoveryMode::AbsoluteConsistency,
        2 => DBRecoveryMode::PointInTime,
        3 => DBRecoveryMode::SkipAnyCorruptedRecord,
        _ => DBRecoveryMode::TolerateCorruptedTailRecords,
    });

    // <https://github.com/facebook/rocksdb/wiki/Track-WAL-in-MANIFEST>
    // "We recommend to set track_and_verify_wals_in_manifest to true for
    // production, it has been enabled in production for the entire database
    // cluster serving the social graph for all Meta apps."
    opts.set_track_and_verify_wals_in_manifest(true);

    opts.set_paranoid_checks(config.rocksdb_paranoid_file_checks);

    opts.set_env(env);

    Ok(opts)
}

/// Configures where the engine's own log goes and how much of it is kept.
///
/// The engine writes these to `LOG` files inside the database directory rather
/// than through phantom's tracing subscriber: routing them into tracing needs
/// a callback logger the crates.io bindings do not expose.
fn set_logging_defaults(opts: &mut Options, config: &Config) {
    let rocksdb_log_level = match config.rocksdb_log_level.as_ref() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "fatal" => LogLevel::Fatal,
        _ => LogLevel::Error,
    };

    opts.set_log_level(rocksdb_log_level);
    opts.set_max_log_file_size(config.rocksdb_max_log_file_size);
    opts.set_log_file_time_to_roll(config.rocksdb_log_time_to_roll);
    opts.set_keep_log_file_num(config.rocksdb_max_log_files);
    opts.set_stats_dump_period_sec(0);
}

/// The thread count for the engine's background work, which the operator may
/// leave at 0 to mean "one per logical CPU".
fn num_threads<T: TryFrom<usize>>(config: &Config) -> Result<T> {
    const MIN_PARALLELISM: usize = 2;

    let requested = if config.rocksdb_parallelism_threads != 0 {
        config.rocksdb_parallelism_threads
    } else {
        utils::available_parallelism()
    };

    utils::math::try_into::<T, usize>(cmp::max(MIN_PARALLELISM, requested))
}

#[cfg(test)]
mod tests {
    use figment::{
        Figment,
        providers::{Format, Toml},
    };

    use super::*;

    fn config(toml: &str) -> Config {
        let toml = format!(
            "[global]\nserver_name = \"phantom.chat\"\ndatabase_path = \"/var/lib/phantom\"\n{toml}"
        );

        Config::new(&Figment::new().merge(Toml::string(&toml).nested())).expect("config is valid")
    }

    #[test]
    fn zero_threads_means_one_per_cpu() {
        let config = config("rocksdb_parallelism_threads = 0\n");

        assert_eq!(
            num_threads::<usize>(&config).expect("fits"),
            cmp::max(2, utils::available_parallelism())
        );
    }

    /// A single background thread cannot both flush and compact, so the engine
    /// is never given fewer than two however few are asked for.
    #[test]
    fn one_thread_is_raised_to_the_minimum() {
        let config = config("rocksdb_parallelism_threads = 1\n");

        assert_eq!(num_threads::<usize>(&config).expect("fits"), 2);
    }

    #[test]
    fn a_thread_count_that_does_not_fit_is_an_error() {
        let config = config(&format!("rocksdb_parallelism_threads = {}\n", u32::MAX));

        assert!(num_threads::<i8>(&config).is_err());
    }
}
