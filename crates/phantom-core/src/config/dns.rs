use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Dns {
    #[serde(default = "default_dns_cache_entries")]
    pub dns_cache_entries: u32,

    #[serde(default = "default_dns_min_ttl")]
    pub dns_min_ttl: u64,

    #[serde(default = "default_dns_min_ttl_nxdomain")]
    pub dns_min_ttl_nxdomain: u64,

    #[serde(default = "default_dns_attempts")]
    pub dns_attempts: u16,

    #[serde(default = "default_dns_timeout")]
    pub dns_timeout: u64,

    #[serde(default = "true_fn")]
    pub dns_tcp_fallback: bool,

    #[serde(default)]
    pub query_over_tcp_only: bool,

    #[serde(default = "true_fn")]
    pub query_all_nameservers: bool,

    #[serde(default)]
    pub ip_lookup_strategy: IpLookupStrategy,
}
