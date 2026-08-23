//! Validation applied to a [`Config`] once it has been deserialized.

use ruma::ServerName;
use tracing::{debug, warn};

use super::{Config, DEPRECATED_KEYS};
use crate::{Result, err};

/// Performs [`check`] with the additional constraints that apply when swapping
/// a running server's config for a freshly loaded one.
pub fn reload(old: &Config, new: &Config) -> Result {
    check(new)?;

    if new.server_name != old.server_name {
        return Err(err!(
            "You can't change the server's name from {:?}.",
            old.server_name
        ));
    }

    Ok(())
}

/// Rejects configs that cannot work, and warns about ones that probably are
/// not what the operator intended.
pub fn check(config: &Config) -> Result {
    if config.server_name.is_empty() {
        return Err(err!("`server_name` must be set"));
    }

    if let Err(e) = ServerName::parse(&config.server_name) {
        return Err(err!("`server_name` is not a valid Matrix server name: {e}"));
    }

    if config.get_bind_addrs().is_empty() {
        return Err(err!(
            "`address` and `port` must each name at least one value"
        ));
    }

    // Built and discarded purely to reject a malformed value here, where it can
    // be attributed to the option it came from, rather than at logging setup
    // where the fallback would be silence.
    config.log_filter()?;
    config.span_events()?;

    check_database(config)?;

    warn_deprecated(config);
    warn_unknown_key(config);

    Ok(())
}

/// The database options the engine cannot fall back from. Each of these is
/// rejected here, where the option that caused it can be named, rather than
/// deep inside the engine where the reference implementation panics or quietly
/// substitutes a different value.
fn check_database(config: &Config) -> Result {
    if config.rocksdb_max_log_files == 0 {
        return Err(err!(Config(
            "rocksdb_max_log_files",
            "must be at least 1; the database engine rejects 0"
        )));
    }

    // The engine matches this against four recovery modes and has nothing to
    // map a fifth onto.
    if config.rocksdb_recovery_mode > 3 {
        return Err(err!(Config(
            "rocksdb_recovery_mode",
            "must be 0, 1, 2, or 3, not {}",
            config.rocksdb_recovery_mode
        )));
    }

    // The reference implementation treats an unrecognized algorithm as zstd,
    // so a typo silently gets a database compressed differently than asked.
    if !COMPRESSION_ALGOS.contains(&config.rocksdb_compression_algo.as_str()) {
        return Err(err!(Config(
            "rocksdb_compression_algo",
            "{:?} is not one of {}",
            config.rocksdb_compression_algo,
            COMPRESSION_ALGOS.join(", ")
        )));
    }

    if config.rocksdb_read_only && config.rocksdb_secondary {
        return Err(err!(Config(
            "rocksdb_secondary",
            "cannot be combined with `rocksdb_read_only`; a secondary instance is already \
             read-only"
        )));
    }

    Ok(())
}

/// Compression algorithms the engine can be asked for, in the spelling the
/// config takes.
const COMPRESSION_ALGOS: &[&str] = &["zstd", "zlib", "bz2", "lz4", "lz4hc", "snappy", "none"];

/// Iterates over all the keys in the config file and warns if there is a
/// deprecated key specified
fn warn_deprecated(config: &Config) {
    debug!("Checking for deprecated config keys");

    let mut was_deprecated = false;
    for key in config
        .catchall
        .keys()
        .filter(|key| DEPRECATED_KEYS.iter().any(|s| s == key))
    {
        warn!("Config parameter \"{key}\" is deprecated, ignoring.");
        was_deprecated = true;
    }

    if was_deprecated {
        warn!(
            "Read the phantom config documentation and check your configuration if any new \
             configuration parameters should be adjusted"
        );
    }
}

/// Iterates over all the catchall keys (unknown config options) and warns if
/// there are any.
fn warn_unknown_key(config: &Config) {
    debug!("Checking for unknown config keys");

    for key in config
        .catchall
        .keys()
        .filter(|key| !DEPRECATED_KEYS.iter().any(|s| s == key))
    {
        warn!("Config parameter \"{key}\" is unknown to phantom, ignoring.");
    }
}
