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

    /// Enables OTLP span export for Jaeger-compatible tracing.
    ///
    /// Only a build with the `perf_measurements` feature installs the
    /// OpenTelemetry layer; elsewhere this has no effect. `jaeger_filter`
    /// selects the exported spans.
    ///
    /// default: false
    #[serde(default)]
    pub allow_jaeger: bool,

    /// Filter directives selecting the spans exported to Jaeger, in the same
    /// syntax as `log`. Debug builds default to "trace,h2=off".
    ///
    /// default: "info"
    #[serde(default = "default_jaeger_filter")]
    pub jaeger_filter: String,

    /// Collects a folded stack trace profile of tracing spans with
    /// tracing_flame, in a build with the `perf_measurements` feature. The
    /// profile can be visualized with inferno[1], speedscope[2], or a number
    /// of other tools.
    ///
    /// [1]: https://github.com/jonhoo/inferno
    /// [2]: www.speedscope.app
    ///
    /// default: false
    #[serde(default)]
    pub tracing_flame: bool,

    /// Filter directives selecting the spans profiled by `tracing_flame`, in
    /// the same syntax as `log`. Debug builds default to "trace,h2=off".
    ///
    /// default: "info"
    #[serde(default = "default_tracing_flame_filter")]
    pub tracing_flame_filter: String,

    /// File the `tracing_flame` profile is written to.
    ///
    /// default: "./tracing.folded"
    #[serde(default = "default_tracing_flame_output_path")]
    pub tracing_flame_output_path: String,
}
