pub mod cache;
pub mod destination;
pub mod dns;
pub mod lookup;
#[cfg(test)]
mod tests;
mod well_known;

use std::sync::Arc;

use arrayvec::ArrayString;
use async_trait::async_trait;
use phantom_core::{Result, server::Server, sync::MutexMap};

use self::{cache::Cache, dns::Resolver};
use crate::{Dep, client};

pub struct Service {
    pub cache: Arc<Cache>,
    pub resolver: Arc<Resolver>,

    resolving: Resolving,
    services: Services,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
}

type Resolving = MutexMap<NameBuf, ()>;

type NameBuf = ArrayString<256>;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let cache = Cache::new(&args);

        Ok(Arc::new(Self {
            cache: cache.clone(),
            resolver: Resolver::build(args.server, cache)?,
            resolving: MutexMap::new(),
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>("client"),
            },
        }))
    }

    async fn clear_cache(&self) {
        self.resolver.clear_cache();
        self.cache.clear().await;
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}
