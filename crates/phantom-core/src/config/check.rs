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

    warn_deprecated(config);
    warn_unknown_key(config);

    Ok(())
}

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
