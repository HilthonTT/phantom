use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Database {
    pub database_path: PathBuf,

    pub database_backup_path: Option<PathBuf>,

    #[serde(default = "default_database_backups_to_keep")]
    pub database_backups_to_keep: i16,

    #[serde(default = "default_db_cache_capacity_mb")]
    pub db_cache_capacity_mb: f64,

    #[serde(default = "default_db_write_buffer_capacity_mb")]
    pub db_write_buffer_capacity_mb: f64,

    #[serde(default = "default_cache_capacity_modifier")]
    pub cache_capacity_modifier: f64,

    #[serde(default = "default_auth_chain_cache_capacity")]
    pub auth_chain_cache_capacity: u32,

    #[serde(default = "default_space_hierarchy_cache_capacity")]
    pub space_hierarchy_cache_capacity: u32,

    #[serde(default = "default_stateinfo_cache_capacity")]
    pub stateinfo_cache_capacity: u32,

    #[serde(default = "default_db_pool_workers")]
    pub db_pool_workers: usize,

    #[serde(default = "default_db_pool_workers_limit")]
    pub db_pool_workers_limit: usize,

    #[serde(default = "default_db_pool_queue_mult")]
    pub db_pool_queue_mult: usize,

    #[serde(default = "true_fn")]
    pub db_pool_affinity: bool,

    #[serde(default = "default_stream_width_scale")]
    pub stream_width_scale: f32,

    #[serde(default = "default_stream_amplification")]
    pub stream_amplification: usize,

    #[serde(default)]
    pub rocksdb_atomic_flush: bool,

    #[serde(default = "true_fn")]
    pub rocksdb_bottommost_compression: bool,

    #[serde(default = "default_rocksdb_compression_level")]
    pub rocksdb_bottommost_compression_level: i32,

    #[serde(default = "true_fn")]
    pub rocksdb_checksums: bool,

    #[serde(default = "true_fn")]
    pub rocksdb_compaction: bool,

    #[serde(default = "true_fn")]
    pub rocksdb_compaction_ioprio_idle: bool,

    #[serde(default)]
    pub rocksdb_compaction_prio_idle: bool,

    #[serde(default = "default_rocksdb_compression_algo")]
    pub rocksdb_compression_algo: String,

    #[serde(default = "default_rocksdb_compression_level")]
    pub rocksdb_compression_level: i32,

    #[serde(default = "true_fn")]
    pub rocksdb_direct_io: bool,

    #[serde(default = "default_rocksdb_log_level")]
    pub rocksdb_log_level: String,

    #[serde(default)]
    pub rocksdb_log_time_to_roll: usize,

    #[serde(default = "default_rocksdb_max_log_file_size")]
    pub rocksdb_max_log_file_size: usize,

    #[serde(default = "default_rocksdb_max_log_files")]
    pub rocksdb_max_log_files: usize,

    #[serde(default)]
    pub rocksdb_optimize_for_spinning_disks: bool,

    #[serde(default)]
    pub rocksdb_parallelism_threads: usize,

    #[serde(default)]
    pub rocksdb_paranoid_file_checks: bool,

    #[serde(default)]
    pub rocksdb_read_only: bool,

    #[serde(default = "default_rocksdb_recovery_mode")]
    pub rocksdb_recovery_mode: u8,

    #[serde(default)]
    pub rocksdb_repair: bool,

    #[serde(default)]
    pub rocksdb_secondary: bool,

    #[serde(default = "default_rocksdb_stats_level")]
    pub rocksdb_stats_level: u8,
}
