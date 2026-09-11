use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Logging {
    #[serde(default)]
    pub allow_metrics: bool,

    #[serde(default = "default_log")]
    pub log: String,

    #[serde(default = "true_fn", alias = "log_colours")]
    pub log_colors: bool,

    #[serde(default = "default_log_span_events")]
    pub log_span_events: String,

    #[serde(default = "true_fn")]
    pub log_filter_regex: bool,

    #[serde(default)]
    pub log_thread_ids: bool,

    #[serde(default = "default_login_token_ttl")]
    pub login_token_ttl: u64,
}
