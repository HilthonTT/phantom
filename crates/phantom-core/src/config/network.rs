//! Outbound HTTP: timeouts, connection pools, and compression.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Network {
    /// CIDR ranges phantom will not send an outbound request to, which is
    /// what keeps a URL preview or a media fetch from being aimed at the
    /// network this server is on.
    ///
    /// This is enforced in phantom, so it is a backstop rather than a
    /// boundary: a firewall is what actually contains the server. Requests
    /// through a proxy are not checked against it. Set it to `[]` to disable.
    ///
    /// The default covers the loopback, private, link-local, shared-address,
    /// documentation, benchmarking, multicast and unique-local ranges of both
    /// address families.
    ///
    /// default: ["127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12",
    /// "192.168.0.0/16", "100.64.0.0/10", "192.0.0.0/24", "169.254.0.0/16",
    /// "192.88.99.0/24", "198.18.0.0/15", "192.0.2.0/24", "198.51.100.0/24",
    /// "203.0.113.0/24", "224.0.0.0/4", "::1/128", "fe80::/10", "fc00::/7",
    /// "2001:db8::/32", "ff00::/8", "fec0::/10"]
    #[serde(default = "default_ip_range_denylist")]
    pub ip_range_denylist: Vec<String>,

    /// Proxy the outbound requests go through.
    ///
    /// `"none"` sends them directly. A table under `[global.proxy]` either
    /// proxies everything:
    ///
    /// ```toml
    /// [global.proxy]
    /// global = { url = "socks5h://localhost:9050" }
    /// ```
    ///
    /// or proxies the domains that match a rule, which is what reaching
    /// `.onion` servers over Tor while everything else goes direct looks
    /// like:
    ///
    /// ```toml
    /// [[global.proxy.by_domain]]
    /// url = "socks5h://localhost:9050"
    /// include = ["*.onion", "matrix.myspecial.onion"]
    /// exclude = ["*.myspecial.onion"]
    /// ```
    ///
    /// An empty `include` reads as `["*"]`. Where a domain matches both
    /// lists, the more specific rule decides — above, the proxy is used for
    /// `ordinary.onion` and `matrix.myspecial.onion`, but not for
    /// `hello.myspecial.onion`.
    ///
    /// Note that `ip_range_denylist` is not applied to a proxied request.
    ///
    /// default: "none"
    #[serde(default)]
    pub proxy: ProxyConfig,

    /// Seconds phantom waits to establish a connection, for the requests that
    /// have no timeout of their own: URL previews and the announcement check.
    ///
    /// default: 10
    #[serde(default = "default_request_conn_timeout")]
    pub request_conn_timeout: u64,

    /// Seconds phantom waits for more data on one of those connections before
    /// giving up on the response.
    ///
    /// default: 35
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,

    /// Seconds one of those requests may take in total. Deliberately far
    /// above `request_timeout`: this is the backstop that stops a request
    /// from living forever, not the one that should normally fire.
    ///
    /// default: 320
    #[serde(default = "default_request_total_timeout")]
    pub request_total_timeout: u64,

    /// Seconds an unused connection is kept in the pool for those requests.
    ///
    /// default: 5
    #[serde(default = "default_request_idle_timeout")]
    pub request_idle_timeout: u64,

    /// Unused connections kept per host for those requests. One is usually
    /// right: the next request to a host can reuse the connection the last
    /// one left.
    ///
    /// default: 1
    #[serde(default = "default_request_idle_per_host")]
    pub request_idle_per_host: u16,

    /// Seconds phantom waits to connect while resolving another server's
    /// `.well-known/matrix/server`.
    ///
    /// default: 6
    #[serde(default = "default_well_known_conn_timeout")]
    pub well_known_conn_timeout: u64,

    /// Seconds a `.well-known` request may take in total.
    ///
    /// default: 10
    #[serde(default = "default_well_known_timeout")]
    pub well_known_timeout: u64,

    /// Seconds phantom waits for more data from another homeserver.
    ///
    /// Worth keeping high: a join to a large room is one request, and the
    /// remote server may be some time assembling the response.
    ///
    /// default: 300
    #[serde(default = "default_federation_timeout")]
    pub federation_timeout: u64,

    /// Seconds an unused federation connection is kept in the pool.
    ///
    /// default: 25
    #[serde(default = "default_federation_idle_timeout")]
    pub federation_idle_timeout: u64,

    /// Unused federation connections kept per remote server.
    ///
    /// default: 1
    #[serde(default = "default_federation_idle_per_host")]
    pub federation_idle_per_host: u16,

    /// Seconds a transaction the sender pushes to another server may take.
    /// The remote server has to process everything in it before answering.
    ///
    /// default: 180
    #[serde(default = "default_sender_timeout")]
    pub sender_timeout: u64,

    /// Seconds an unused sender connection is kept in the pool.
    ///
    /// default: 180
    #[serde(default = "default_sender_idle_timeout")]
    pub sender_idle_timeout: u64,

    /// Longest a remote server that keeps failing is left alone between
    /// attempts. The wait after a failure doubles each time, starting from
    /// `sender_timeout`, and stops growing here.
    ///
    /// default: 86400
    #[serde(default = "default_sender_retry_backoff_limit")]
    pub sender_retry_backoff_limit: u64,

    /// Seconds the sender waits for its in-flight transactions to finish at
    /// shutdown before giving up on them. Nothing is lost either way: what
    /// was not acknowledged is sent again at the next start.
    ///
    /// default: 5
    #[serde(default = "default_sender_shutdown_timeout")]
    pub sender_shutdown_timeout: u64,

    /// Worker tasks the sender spreads its destinations over. Each remote
    /// server always lands on the same worker, so ordering to one server is
    /// kept. Never more than the runtime has threads.
    ///
    /// 0 runs a single worker.
    ///
    /// default: 0
    #[serde(default)]
    pub sender_workers: usize,

    /// Send the transactions that were still in flight when the server last
    /// stopped as soon as it starts. Off, they wait until there is something
    /// new to send to that server.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub startup_netburst: bool,

    /// Events per remote server the startup burst may carry; the rest are
    /// dropped. Servers that were unreachable for a long time can have a lot
    /// queued, and sending all of it at once is rarely wanted. -1 keeps
    /// everything.
    ///
    /// default: 50
    #[serde(default = "default_startup_netburst_keep")]
    pub startup_netburst_keep: i64,

    /// Seconds a request to an appservice may take. Appservices usually sit
    /// on the same network, so this is about the work they do rather than the
    /// distance.
    ///
    /// default: 35
    #[serde(default = "default_appservice_timeout")]
    pub appservice_timeout: u64,

    /// Seconds an unused appservice connection is kept in the pool.
    ///
    /// default: 300
    #[serde(default = "default_appservice_idle_timeout")]
    pub appservice_idle_timeout: u64,

    /// Seconds an unused push gateway connection is kept in the pool.
    ///
    /// default: 15
    #[serde(default = "default_pusher_idle_timeout")]
    pub pusher_idle_timeout: u64,

    /// Accept and decompress gzip-encoded responses.
    ///
    /// Compression on a TLS connection can leak plaintext to someone watching
    /// the sizes; see https://en.wikipedia.org/wiki/BREACH. Off unless the
    /// bandwidth matters more.
    #[serde(default)]
    pub gzip_compression: bool,

    /// Accept and decompress brotli-encoded responses. See
    /// `gzip_compression`.
    #[serde(default)]
    pub brotli_compression: bool,

    /// Accept and decompress zstd-encoded responses. See `gzip_compression`.
    #[serde(default)]
    pub zstd_compression: bool,

    /// Skip TLS certificate validation on every outbound request.
    ///
    /// There is no safe use of this outside a lab: it hands anyone who can
    /// intercept the connection everything that goes over it, federation
    /// traffic included. `validate` refuses to let it pass quietly.
    #[serde(default)]
    pub allow_invalid_tls_certificates: bool,
}
