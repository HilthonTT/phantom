use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Network {
    #[serde(default = "default_ip_range_denylist")]
    pub ip_range_denylist: Vec<String>,

    #[serde(default)]
    pub proxy: ProxyConfig,

    #[serde(default = "default_request_conn_timeout")]
    pub request_conn_timeout: u64,

    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    #[serde(default = "default_request_total_timeout")]
    pub request_total_timeout: u64,

    #[serde(default = "default_request_idle_timeout")]
    pub request_idle_timeout: u64,

    #[serde(default = "default_request_idle_per_host")]
    pub request_idle_per_host: u16,

    #[serde(default = "default_well_known_conn_timeout")]
    pub well_known_conn_timeout: u64,

    #[serde(default = "default_well_known_timeout")]
    pub well_known_timeout: u64,

    #[serde(default = "default_federation_timeout")]
    pub federation_timeout: u64,

    #[serde(default = "default_federation_idle_timeout")]
    pub federation_idle_timeout: u64,

    #[serde(default = "default_federation_idle_per_host")]
    pub federation_idle_per_host: u16,

    #[serde(default = "default_sender_timeout")]
    pub sender_timeout: u64,

    #[serde(default = "default_sender_idle_timeout")]
    pub sender_idle_timeout: u64,

    #[serde(default = "default_sender_retry_backoff_limit")]
    pub sender_retry_backoff_limit: u64,

    #[serde(default = "default_sender_retry_grace")]
    pub sender_retry_grace: u64,

    #[serde(default = "default_feds_max_width")]
    pub feds_max_width: usize,

    #[serde(default = "default_feds_timeout")]
    pub feds_timeout: u64,

    #[serde(default = "default_sender_shutdown_timeout")]
    pub sender_shutdown_timeout: u64,

    #[serde(default)]
    pub sender_workers: usize,

    #[serde(default = "true_fn")]
    pub startup_netburst: bool,

    #[serde(default = "default_startup_netburst_keep")]
    pub startup_netburst_keep: i64,

    #[serde(default = "default_appservice_timeout")]
    pub appservice_timeout: u64,

    #[serde(default = "default_appservice_idle_timeout")]
    pub appservice_idle_timeout: u64,

    #[serde(default = "default_pusher_idle_timeout")]
    pub pusher_idle_timeout: u64,

    #[serde(default)]
    pub gzip_compression: bool,

    #[serde(default)]
    pub brotli_compression: bool,

    #[serde(default)]
    pub zstd_compression: bool,

    #[serde(default)]
    pub allow_invalid_tls_certificates: bool,
}
