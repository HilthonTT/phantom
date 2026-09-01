use super::*;

pub(super) fn true_fn() -> bool {
    true
}

pub(super) fn default_log() -> String {
    cfg!(debug_assertions)
        .then_some("debug")
        .unwrap_or("info")
        .to_owned()
}

pub(super) fn default_log_span_events() -> String {
    "none".to_owned()
}

pub(super) fn default_address() -> ListeningAddr {
    ListeningAddr {
        addrs: Right(vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]),
    }
}

pub(super) fn default_port() -> ListeningPort {
    ListeningPort { ports: Left(8008) }
}

pub(super) fn default_database_backups_to_keep() -> i16 {
    1
}

pub(super) fn default_db_cache_capacity_mb() -> f64 {
    128.0 + parallelism_scaled_f64(64.0)
}

pub(super) fn default_db_write_buffer_capacity_mb() -> f64 {
    48.0 + parallelism_scaled_f64(4.0)
}

pub(super) fn default_cache_capacity_modifier() -> f64 {
    1.0
}

pub(super) fn default_auth_chain_cache_capacity() -> u32 {
    parallelism_scaled_u32(10_000).saturating_add(100_000)
}

pub(super) fn default_stateinfo_cache_capacity() -> u32 {
    parallelism_scaled_u32(10).saturating_add(100)
}

pub(super) fn default_openid_token_ttl() -> u64 {
    60 * 60
}

pub(super) fn default_login_token_ttl() -> u64 {
    2 * 60 * 1000
}

pub(super) fn default_presence_idle_timeout_s() -> u64 {
    5 * 60
}

pub(super) fn default_presence_offline_timeout_s() -> u64 {
    30 * 60
}

pub(super) fn default_db_pool_workers() -> usize {
    32
}

pub(super) fn default_db_pool_workers_limit() -> usize {
    64
}

pub(super) fn default_db_pool_queue_mult() -> usize {
    4
}

pub(super) fn default_stream_width_scale() -> f32 {
    1.0
}

pub(super) fn default_stream_amplification() -> usize {
    1024
}

/// RocksDB reads 32767 as "use whatever this algorithm calls its default
/// level", since the range of valid levels differs per algorithm. It is the
/// sentinel phantom watches for before substituting a per-column level of its
/// own.
pub(super) fn default_rocksdb_compression_level() -> i32 {
    32767
}

pub(super) fn default_rocksdb_compression_algo() -> String {
    "zstd".to_owned()
}

pub(super) fn default_rocksdb_log_level() -> String {
    "error".to_owned()
}

pub(super) fn default_rocksdb_max_log_file_size() -> usize {
    4 * 1024 * 1024
}

pub(super) fn default_rocksdb_max_log_files() -> usize {
    3
}

pub(super) fn default_rocksdb_recovery_mode() -> u8 {
    1
}

pub(super) fn default_rocksdb_stats_level() -> u8 {
    1
}

pub(super) fn default_trusted_servers() -> Vec<OwnedServerName> {
    vec![OwnedServerName::try_from("matrix.org").expect("matrix.org is a valid server name")]
}

pub(super) fn default_trusted_server_batch_size() -> usize {
    256
}

pub(super) fn default_turn_ttl() -> u64 {
    60 * 60 * 24
}

pub(super) fn default_notification_push_path() -> String {
    "/_matrix/push/v1/notify".to_owned()
}

pub(super) fn default_url_preview_max_spider_size() -> usize {
    256_000
}

/// Every range that has no business being reached from the public internet:
/// loopback, the three private ranges, shared address space, IETF protocol
/// assignments, link-local, 6to4 relay anycast, benchmarking, the three
/// documentation ranges, and multicast — then the v6 equivalents.
pub(super) fn default_ip_range_denylist() -> Vec<String> {
    [
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "192.0.0.0/24",
        "169.254.0.0/16",
        "192.88.99.0/24",
        "198.18.0.0/15",
        "192.0.2.0/24",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "224.0.0.0/4",
        "::1/128",
        "fe80::/10",
        "fc00::/7",
        "2001:db8::/32",
        "ff00::/8",
        "fec0::/10",
    ]
    .map(ToOwned::to_owned)
    .to_vec()
}

pub(super) fn default_request_conn_timeout() -> u64 {
    10
}

pub(super) fn default_request_timeout() -> u64 {
    35
}

pub(super) fn default_request_total_timeout() -> u64 {
    320
}

pub(super) fn default_request_idle_timeout() -> u64 {
    5
}

pub(super) fn default_request_idle_per_host() -> u16 {
    1
}

pub(super) fn default_well_known_conn_timeout() -> u64 {
    6
}

pub(super) fn default_well_known_timeout() -> u64 {
    10
}

pub(super) fn default_federation_timeout() -> u64 {
    300
}

pub(super) fn default_federation_idle_timeout() -> u64 {
    25
}

pub(super) fn default_federation_idle_per_host() -> u16 {
    1
}

pub(super) fn default_sender_timeout() -> u64 {
    180
}

pub(super) fn default_sender_idle_timeout() -> u64 {
    180
}

pub(super) fn default_appservice_timeout() -> u64 {
    35
}

pub(super) fn default_appservice_idle_timeout() -> u64 {
    300
}

pub(super) fn default_pusher_idle_timeout() -> u64 {
    15
}

pub(super) fn default_dns_cache_entries() -> u32 {
    32768
}

pub(super) fn default_dns_min_ttl() -> u64 {
    60 * 60 * 3
}

pub(super) fn default_dns_min_ttl_nxdomain() -> u64 {
    60 * 60 * 24 * 3
}

pub(super) fn default_dns_attempts() -> u16 {
    10
}

pub(super) fn default_dns_timeout() -> u64 {
    10
}

/// Scales a per-core figure by the parallelism actually available to this
/// process, which is what the memory defaults above are expressed in.
pub(super) fn parallelism_scaled_f64(val: f64) -> f64 {
    #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
    let cores = crate::sys::compute::available_parallelism() as f64;

    val * cores
}

/// [`parallelism_scaled_f64`] for the cache capacities, which are counts of
/// entries rather than megabytes.
pub(super) fn parallelism_scaled_u32(val: u32) -> u32 {
    let cores = crate::sys::compute::available_parallelism();

    usize::try_from(val)
        .map(|val| val.saturating_mul(cores))
        .map_or(u32::MAX, |val| u32::try_from(val).unwrap_or(u32::MAX))
}
