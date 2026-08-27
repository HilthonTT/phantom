//! Turning a Matrix server name into an address to connect to.
//!
//! A server name is not a hostname. Resolving one is the procedure in
//! [the spec][spec]: an IP literal is used as-is, a name carrying a port is
//! used as-is, and otherwise the name's `.well-known/matrix/server` is asked
//! first, its SRV records second, and only then is the name itself resolved.
//! [`actual`] walks those steps in order and is the entry point for it.
//!
//! What comes out is cached in the database rather than only in memory: the
//! well-known and SRV lookups are several round trips before the first byte
//! of a federation request can be sent, and the answer is good for hours.
//! [`cache`] holds it, and [`dns`] is what lets reqwest read that cache when
//! it opens the connection.
//!
//! [spec]: https://spec.matrix.org/latest/server-server-api/#resolving-server-names

pub mod actual;
pub mod cache;
pub mod dns;
pub mod fed;
#[cfg(test)]
mod tests;
mod well_known;

use std::sync::Arc;

use arrayvec::ArrayString;
use async_trait::async_trait;
use phantom_core::{Result, server::Server, utils::MutexMap};

use self::{cache::Cache, dns::Resolver};
use crate::{Dep, client};

pub struct Service {
    pub cache: Arc<Cache>,
    pub resolver: Arc<Resolver>,

    /// One resolution per server name at a time. Without it, a burst of
    /// requests to a server nothing has talked to yet would each run the
    /// whole well-known and SRV sequence before any of them had an answer to
    /// cache.
    resolving: Resolving,
    services: Services,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
}

type Resolving = MutexMap<NameBuf, ()>;

/// A server name is capped at 255 bytes by the grammar, so the key this is
/// deduplicated on never needs to allocate.
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
        crate::service::make_name(std::module_path!())
    }
}
