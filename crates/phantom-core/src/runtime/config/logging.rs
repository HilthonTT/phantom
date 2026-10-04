use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Logging {
    #[serde(default)]
    pub allow_metrics: bool,

    #[serde(default = "default_log")]
    pub log: String,

    /// Whether to colour console log output with ANSI escape codes.
    ///
    /// Colours are always turned off when logging to journald, see
    /// `log_to_journald`.
    ///
    /// default: true
    #[serde(default = "true_fn", alias = "log_colours")]
    pub log_colors: bool,

    /// Treat the console output as going to the systemd journal, which
    /// turns off ANSI colours and writes to stderr.
    ///
    /// This is detected automatically when running as a systemd service, so
    /// it only needs to be set when that detection fails.
    ///
    /// default: false
    #[serde(default)]
    pub log_to_journald: bool,

    #[serde(default = "default_log_span_events")]
    pub log_span_events: String,

    #[serde(default = "true_fn")]
    pub log_filter_regex: bool,

    #[serde(default)]
    pub log_thread_ids: bool,

    #[serde(default = "default_login_token_ttl")]
    pub login_token_ttl: u64,
}
