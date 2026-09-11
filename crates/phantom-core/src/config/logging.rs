//! What the server logs, and how.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Logging {
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

    /// Milliseconds a login token stays valid for.
    ///
    /// This is the `m.login.token` handed out to complete a login started
    /// elsewhere, so it is spent within seconds of being issued; the spec caps
    /// it at five minutes.
    ///
    /// default: 120000
    #[serde(default = "default_login_token_ttl")]
    pub login_token_ttl: u64,
}
