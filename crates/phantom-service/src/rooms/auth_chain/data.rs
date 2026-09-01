use std::{
    mem::size_of,
    sync::{Arc, Mutex},
};

use lru_cache::LruCache;
use phantom_core::{Err, Result, bytes::u64_from_u8, err, math::usize_from_f64};
use phantom_database::Map;

use crate::rooms::short::ShortEventId;

pub(super) struct Data {
    shorteventid_authchain: Arc<Map>,
    pub(super) auth_chain_cache: Mutex<LruCache<Vec<ShortEventId>, Arc<[ShortEventId]>>>,
}

impl Data {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let db = &args.db;
        let config = &args.server.config;
        let cache_size = f64::from(config.auth_chain_cache_capacity);
        let cache_size =
            usize_from_f64(cache_size * config.cache_capacity_modifier).expect("valid cache size");
        Self {
            shorteventid_authchain: db["shorteventid_authchain"].clone(),
            auth_chain_cache: Mutex::new(LruCache::new(cache_size)),
        }
    }

    /// The cached chain for these starting events, from memory or, for a
    /// single event, from the column behind it.
    pub(super) async fn cached_auth_chain(
        &self,
        key: &[ShortEventId],
    ) -> Result<Arc<[ShortEventId]>> {
        debug_assert!(!key.is_empty(), "auth_chain key must not be empty");

        if let Some(result) = self
            .auth_chain_cache
            .lock()
            .expect("cache locked")
            .get_mut(key)
        {
            return Ok(Arc::clone(result));
        }

        if key.len() != 1 {
            return Err!(Request(NotFound("auth_chain not cached")));
        }

        let chain = self
            .shorteventid_authchain
            .qry(&key[0])
            .await
            .map_err(|_| err!(Request(NotFound("auth_chain not found"))))?;

        let chain = chain
            .chunks_exact(size_of::<u64>())
            .map(u64_from_u8)
            .collect::<Arc<[ShortEventId]>>();

        self.auth_chain_cache
            .lock()
            .expect("cache locked")
            .insert(vec![key[0]], Arc::clone(&chain));

        Ok(chain)
    }

    /// Caches the chain in memory, and in the database if it is one event's.
    pub(super) fn cache_auth_chain(&self, key: Vec<ShortEventId>, auth_chain: Arc<[ShortEventId]>) {
        debug_assert!(!key.is_empty(), "auth_chain key must not be empty");

        if key.len() == 1 {
            let key = key[0].to_be_bytes();
            let val = auth_chain
                .iter()
                .flat_map(|s| s.to_be_bytes().to_vec())
                .collect::<Vec<u8>>();

            self.shorteventid_authchain.insert(&key, &val).ok();
        }

        self.auth_chain_cache
            .lock()
            .expect("cache locked")
            .insert(key, auth_chain);
    }
}
