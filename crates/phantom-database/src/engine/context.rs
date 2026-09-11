use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use phantom_core::{Result, debug, math::usize_from_f64, server::Server};
use rocksdb::{Cache, Env, LruCacheOptions};

use crate::engine::error::or_else;

pub struct Context {
    pub(crate) col_cache: Mutex<BTreeMap<String, Cache>>,
    pub(crate) row_cache: Mutex<Cache>,
    pub(crate) env: Mutex<Env>,
    pub(crate) server: Arc<Server>,
}

static CONTEXTS: Mutex<usize> = Mutex::new(0);

impl Context {
    pub(crate) const SHARED_CACHE: &'static str = "Shared";

    pub fn new(server: &Arc<Server>) -> Result<Arc<Self>> {
        let config = &server.config;
        let cache_capacity_bytes = config.database.db_cache_capacity_mb * 1024.0 * 1024.0;

        let col_shard_bits = 7;
        let col_cache_capacity_bytes = usize_from_f64(cache_capacity_bytes * 0.50)?;

        let row_shard_bits = 7;
        let row_cache_capacity_bytes = usize_from_f64(cache_capacity_bytes * 0.50)?;

        let mut row_cache_opts = LruCacheOptions::default();
        row_cache_opts.set_num_shard_bits(row_shard_bits);
        row_cache_opts.set_capacity(row_cache_capacity_bytes);
        let row_cache = Cache::new_lru_cache_opts(&row_cache_opts);

        let mut col_cache_opts = LruCacheOptions::default();
        col_cache_opts.set_num_shard_bits(col_shard_bits);
        col_cache_opts.set_capacity(col_cache_capacity_bytes);
        let col_cache = Cache::new_lru_cache_opts(&col_cache_opts);
        let col_cache: BTreeMap<_, _> = [(Self::SHARED_CACHE.to_owned(), col_cache)].into();

        let mut contexts = CONTEXTS.lock().expect("locked");

        let mut env = Env::new().or_else(or_else)?;

        if config.database.rocksdb_compaction_prio_idle {
            env.lower_thread_pool_cpu_priority();
        }

        if config.database.rocksdb_compaction_ioprio_idle {
            env.lower_thread_pool_io_priority();
        }

        *contexts = contexts.saturating_add(1);

        Ok(Arc::new(Self {
            col_cache: col_cache.into(),
            row_cache: row_cache.into(),
            env: env.into(),
            server: server.clone(),
        }))
    }
}

impl Drop for Context {
    #[cold]
    fn drop(&mut self) {
        let mut contexts = CONTEXTS.lock().expect("locked");
        *contexts = contexts.saturating_sub(1);

        if *contexts > 0 {
            debug!("Leaving background threads to the databases still open");
            return;
        }

        let mut env = self.env.lock().expect("locked");

        debug!("Shutting down background threads");
        env.set_high_priority_background_threads(0);
        env.set_low_priority_background_threads(0);
        env.set_bottom_priority_background_threads(0);
        env.set_background_threads(0);

        debug!("Joining background threads...");
        env.join_all_threads();
    }
}
