//! What a service is, and what it is handed when it is built.
//!
//! One trait, implemented once per area of the server's behaviour. Everything
//! a service needs from the rest of the process arrives through [`Args`], so
//! adding one later does not touch every implementation.

use std::{any::Any, fmt::Write, sync::Arc};

use async_trait::async_trait;
use phantom_core::{Result, server::Server};
use phantom_database::Database;

use super::registry::{Dep, Map, require};

/// Abstract interface for a Service
#[async_trait]
pub trait Service: Any + Send + Sync {
    /// Implement the construction of the service instance. Services are
    /// generally singletons so expect this to only be called once for a
    /// service type. Note that it may be called again after a server reload,
    /// but the prior instance will have been dropped first. Failure will
    /// shutdown the server with an error.
    ///
    /// The reference returns `Arc<impl Service>` here. That is opaque to the
    /// caller, so what comes back cannot be stored in a field of the service's
    /// own type, and an implementation returning `Arc<Self>` — which is what
    /// every one of them wants to do — is a `refining_impl_trait` warning.
    fn build(args: Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized;

    /// Implement the service's worker loop. The service manager spawns a
    /// task and calls this function after all services have been built.
    async fn worker(self: Arc<Self>) -> Result<()> {
        Ok(())
    }

    /// Interrupt the service. This is sent to initiate a graceful shutdown.
    /// The service worker should return from its work loop.
    fn interrupt(&self) {}

    /// Clear any caches or similar runtime state.
    async fn clear_cache(&self) {}

    /// Memory usage report in a markdown string.
    async fn memory_usage(&self, _out: &mut (dyn Write + Send)) -> Result {
        Ok(())
    }

    /// The service's name, which is what it is registered and looked up
    /// under. Implementations spell it as
    /// `crate::make_name(std::module_path!())` so that the name and the module
    /// cannot drift apart.
    fn name(&self) -> &str;

    /// Return true if the service worker opts out of the tokio cooperative
    /// budgeting. This can reduce tail latency at the risk of event loop
    /// starvation.
    fn unconstrained(&self) -> bool {
        false
    }
}

/// Args are passed to `Service::build` when a service is constructed. This
/// allows for arguments to change with limited impact to the many services.
pub struct Args<'a> {
    pub server: &'a Arc<Server>,
    pub db: &'a Arc<Database>,
    pub service: &'a Arc<Map>,
}

impl<'a> Args<'a> {
    /// Create a lazy-reference to a service when constructing another Service.
    #[inline]
    pub fn depend<T: Service>(&'a self, name: &'static str) -> Dep<T> {
        Dep::<T>::new(self.service, name)
    }

    /// Create a reference immediately to a service when constructing another
    /// Service. The other service must be constructed.
    #[inline]
    pub fn require<T: Service>(&'a self, name: &str) -> Arc<T> {
        require::<T>(self.service, name)
    }
}
