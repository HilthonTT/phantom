//! Name resolution.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Dns {
    /// Entries the in-process DNS cache holds. Entries vary in size, so raise
    /// this carefully. Lower it only where a real external cache sits in
    /// front of phantom — systemd-resolved is not one.
    ///
    /// default: 32768
    #[serde(default = "default_dns_cache_entries")]
    pub dns_cache_entries: u32,

    /// Seconds a resolved name is held even if its record says less.
    ///
    /// default: 10800
    #[serde(default = "default_dns_min_ttl")]
    pub dns_min_ttl: u64,

    /// Seconds an NXDOMAIN is held. Three days by default, and deliberately:
    /// a name that does not resolve is almost never about to, and rechecking
    /// each one is what makes a server with dead remotes in its rooms slow.
    ///
    /// default: 259200
    #[serde(default = "default_dns_min_ttl_nxdomain")]
    pub dns_min_ttl_nxdomain: u64,

    /// Attempts made at a nameserver before the query is given up on.
    ///
    /// default: 10
    #[serde(default = "default_dns_attempts")]
    pub dns_attempts: u16,

    /// Seconds phantom waits for a nameserver to answer. Recursive queries
    /// take seconds on some domains, so a low value here reads as a DNS
    /// outage.
    ///
    /// default: 10
    #[serde(default = "default_dns_timeout")]
    pub dns_timeout: u64,

    /// Retry a query over TCP when the UDP one errors or is truncated.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub dns_tcp_fallback: bool,

    /// Query the configured nameservers over TCP only. Some container
    /// networks need this, where UDP replies are truncated or dropped.
    #[serde(default)]
    pub query_over_tcp_only: bool,

    /// Keep asking the remaining nameservers after one says the name does not
    /// exist, rather than trusting the first negative answer.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub query_all_nameservers: bool,

    /// Which address records to look up, and in what order.
    ///
    /// One of "ipv4-only", "ipv6-only", "ipv4-and-ipv6" (both at once, first
    /// answer wins), "ipv6-then-ipv4", or "ipv4-then-ipv6".
    ///
    /// On a host with no IPv6 route, "ipv4-only" saves a lookup whose answer
    /// could never be connected to.
    ///
    /// default: "ipv4-then-ipv6"
    #[serde(default)]
    pub ip_lookup_strategy: IpLookupStrategy,
}
