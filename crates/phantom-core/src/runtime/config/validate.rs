use ruma::ServerName;
use tracing::{debug, warn};

use super::{Config, DEPRECATED_KEYS};
use crate::{Result, err};

pub fn validate_reload(old: &Config, new: &Config) -> Result {
    validate(new)?;

    if new.server_name != old.server_name {
        return Err(err!(
            "You can't change the server's name from {:?}.",
            old.server_name
        ));
    }

    Ok(())
}

pub fn validate(config: &Config) -> Result {
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

    config.log_filter()?;
    config.span_events()?;

    check_database(config)?;
    check_registration(config)?;

    warn_url_previews(config);
    warn_insecure(config);
    warn_deprecated(config);
    warn_unknown_key(config);

    Ok(())
}

fn check_database(config: &Config) -> Result {
    if config.database.rocksdb_max_log_files == 0 {
        return Err(err!(Config(
            "rocksdb_max_log_files",
            "must be at least 1; the database engine rejects 0"
        )));
    }

    if config.database.rocksdb_recovery_mode > 3 {
        return Err(err!(Config(
            "rocksdb_recovery_mode",
            "must be 0, 1, 2, or 3, not {}",
            config.database.rocksdb_recovery_mode
        )));
    }

    if !COMPRESSION_ALGOS.contains(&config.database.rocksdb_compression_algo.as_str()) {
        return Err(err!(Config(
            "rocksdb_compression_algo",
            "{:?} is not one of {}",
            config.database.rocksdb_compression_algo,
            COMPRESSION_ALGOS.join(", ")
        )));
    }

    if config.database.rocksdb_read_only && config.database.rocksdb_secondary {
        return Err(err!(Config(
            "rocksdb_secondary",
            "cannot be combined with `rocksdb_read_only`; a secondary instance is already \
             read-only"
        )));
    }

    Ok(())
}

const COMPRESSION_ALGOS: &[&str] = &["zstd", "zlib", "bz2", "lz4", "lz4hc", "snappy", "none"];

fn check_registration(config: &Config) -> Result {
    if config.auth.registration_token.as_deref() == Some("") {
        return Err(err!(Config(
            "registration_token",
            "was set to the empty string; unset it instead to require no token"
        )));
    }

    let Some(path) = config.auth.registration_token_file.as_ref() else {
        return Ok(());
    };

    let token = std::fs::read_to_string(path).map_err(|error| {
        err!(Config(
            "registration_token_file",
            "{path:?} could not be read: {error}"
        ))
    })?;

    if token.trim().is_empty() {
        return Err(err!(Config("registration_token_file", "{path:?} is empty")));
    }

    Ok(())
}

fn warn_url_previews(config: &Config) {
    let wildcarded = [
        (
            "url_preview_domain_contains_allowlist",
            &config.media.url_preview_domain_contains_allowlist,
        ),
        (
            "url_preview_domain_explicit_allowlist",
            &config.media.url_preview_domain_explicit_allowlist,
        ),
        (
            "url_preview_url_contains_allowlist",
            &config.media.url_preview_url_contains_allowlist,
        ),
    ]
    .into_iter()
    .filter(|(_, list)| list.iter().any(|entry| entry == "*"));

    for (option, _) in wildcarded {
        warn!(
            "Config parameter \"{option}\" is \"*\", which allows a URL preview to be fetched \
             from any host this server can reach."
        );
    }
}

fn warn_insecure(config: &Config) {
    if config.network.allow_invalid_tls_certificates {
        warn!(
            "Config parameter \"allow_invalid_tls_certificates\" is set. Every outbound \
             connection, federation included, will accept any certificate presented to it. \
             Anyone able to intercept one can read and rewrite it."
        );
    }

    if config.federation.federation_loopback {
        warn!(
            "Config parameter \"federation_loopback\" is set. This server will send federation \
             requests to itself, which outside a development setup is a bug rather than a \
             configuration."
        );
    }
}

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
