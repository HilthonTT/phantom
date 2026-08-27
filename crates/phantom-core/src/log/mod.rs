//! Logging and tracing.

pub mod capture;
pub mod color;
pub mod console;
pub mod debug;
pub mod fmt;
pub mod fmt_span;
mod reload;
mod suppress;
pub mod truncate;

use std::sync::Arc;

pub use tracing::{Level, Subscriber};
pub use tracing_core::{Event, Metadata};
pub use tracing_subscriber::EnvFilter;
use tracing_subscriber::{Layer as _, Registry, layer::SubscriberExt};

pub use self::{
    capture::Capture,
    console::{ConsoleFormat, ConsoleWriter, is_systemd_mode},
    debug::INFO_SPAN_LEVEL,
    reload::{LogLevelReloadHandles, ReloadHandle},
    suppress::Suppress,
    truncate::{TruncatedSlice, slice_truncated},
};
use crate::{Config, Result};

/// Logging subsystem. This is a singleton member of [`crate::Server`] which
/// holds all logging and tracing related state rather than shoving it all in
/// [`crate::Server`] directly.
pub struct Log {
    /// General log level reload handles.
    pub reload: LogLevelReloadHandles,

    /// Tracing capture state for ephemeral/oneshot uses.
    pub capture: Arc<capture::State>,
}

/// Builds the logging subsystem and the subscriber that feeds it.
///
/// The subscriber is returned rather than installed: which of
/// [`tracing::subscriber::set_global_default`] or
/// [`tracing::subscriber::set_default`] to use is the caller's decision, and a
/// library that installs a process-wide subscriber cannot be used twice.
pub fn init(config: &Config) -> Result<(Log, impl Subscriber + Send + Sync + 'static)> {
    let reload = LogLevelReloadHandles::default();

    // One format, used for both the event and its fields, so the two cannot
    // drift apart in configuration.
    let format = ConsoleFormat::new(config);
    let console = tracing_subscriber::fmt::Layer::new()
        .with_span_events(config.span_events()?)
        .event_format(format.clone())
        .fmt_fields(format)
        .with_writer(ConsoleWriter::new(config));

    let (console_filter, console_handle) =
        tracing_subscriber::reload::Layer::new(config.log_filter()?);
    reload.add("console", console_handle);

    let capture = Arc::new(capture::State::new());
    let subscriber = Registry::default()
        .with(console.with_filter(console_filter))
        .with(capture::Layer::new(&capture));

    Ok((Log { reload, capture }, subscriber))
}

// Wrappers for the logging macros. Use these rather than the `tracing` or `log`
// crates directly in project code: the indirection is what lets the level or
// the backend change in one place. The `debug_*` variants in `crate::log::debug`
// are exported to the crate namespace alongside these.

#[macro_export]
macro_rules! event {
    ( $level:expr, $($x:tt)+ ) => { $crate::tracing::event!( $level, $($x)+ ) };
}

#[macro_export]
macro_rules! error {
    ( $($x:tt)+ ) => { $crate::tracing::error!( $($x)+ ) };
}

#[macro_export]
macro_rules! warn {
    ( $($x:tt)+ ) => { $crate::tracing::warn!( $($x)+ ) };
}

#[macro_export]
macro_rules! info {
    ( $($x:tt)+ ) => { $crate::tracing::info!( $($x)+ ) };
}

#[macro_export]
macro_rules! debug {
    ( $($x:tt)+ ) => { $crate::tracing::debug!( $($x)+ ) };
}

#[macro_export]
macro_rules! trace {
    ( $($x:tt)+ ) => { $crate::tracing::trace!( $($x)+ ) };
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use figment::providers::{Format, Toml};

    use super::*;

    fn config(log: &str) -> Config {
        let toml = format!(
            r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            log = "{log}"
            "#
        );

        Config::new(&figment::Figment::new().merge(Toml::string(&toml).nested()))
            .expect("config is valid")
    }

    #[test]
    fn the_subscriber_feeds_captures_and_the_filter_is_reloadable() {
        let (log, subscriber) = init(&config("warn")).expect("initialized");
        let _default = tracing::subscriber::set_default(subscriber);

        assert_eq!(log.reload.names(), ["console"], "the console is reloadable");

        let out = Arc::new(Mutex::new(String::new()));
        let capture = Capture::new(
            &log.capture,
            None::<fn(capture::Data<'_>) -> bool>,
            capture::fmt_markdown(out.clone()),
        );

        let _guard = capture.start();
        tracing::error!("captured line");

        assert!(
            out.lock().expect("locked").contains("captured line"),
            "{}",
            out.lock().expect("locked")
        );
    }

    #[test]
    fn malformed_log_options_are_rejected_when_the_config_loads() {
        // `Config::check` builds both of these, so a value the logging setup
        // could not use never reaches `init` in the first place.
        let cases = [
            ("log = \"phantom=notalevel\"", "log"),
            ("log_span_events = \"cloze\"", "log_span_events"),
        ];

        for (option, expected) in cases {
            let toml = format!(
                r#"
                [global]
                server_name = "phantom.chat"
                database_path = "/var/lib/phantom"
                {option}
                "#
            );

            let error = Config::new(&figment::Figment::new().merge(Toml::string(&toml).nested()))
                .expect_err("rejected");

            assert!(error.message().contains(expected), "{option}: {error}");
        }
    }
}
