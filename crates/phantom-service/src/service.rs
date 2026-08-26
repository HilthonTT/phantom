use std::{
    any::Any,
    collections::BTreeMap,
    fmt::Write,
    ops::Deref,
    sync::{Arc, OnceLock, RwLock, Weak},
};

use async_trait::async_trait;
use phantom_core::{
    Err, Result, err, error::inspect_log, server::Server, strings::SplitInfallible, trace,
};
use phantom_database::Database;

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

    /// Return the name of the service.
    /// i.e. `crate::service::make_name(std::module_path!())`
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

/// Dep is a reference to a service used within another service.
/// Circular-dependencies between services require this indirection.
///
/// This is `Sync` by inference: `Service` is bound `Send + Sync`, so the
/// `OnceLock` and the `Weak` below are too. The reference asserts it with an
/// `unsafe impl` instead, because proving it means walking the whole service
/// graph — every service holding `Dep`s on services that hold `Dep`s back —
/// and that can exhaust the recursion limit. If it ever does here,
/// `#![recursion_limit = "192"]` on this crate is the answer; unsafety is not.
pub struct Dep<T: Service + Send + Sync> {
    dep: OnceLock<Arc<T>>,
    service: Weak<Map>,
    name: &'static str,
}

pub type Map = RwLock<MapType>;
pub type MapType = BTreeMap<MapKey, MapVal>;
pub type MapVal = (Weak<dyn Service>, Weak<dyn Any + Send + Sync>);
pub type MapKey = String;

impl<T: Service + Send + Sync> Deref for Dep<T> {
    type Target = Arc<T>;

    /// Dereference a dependency. The dependency must be ready or panics.
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.dep.get_or_init(
            #[inline(never)]
            || self.init(),
        )
    }
}

impl<T: Service + Send + Sync> Dep<T> {
    #[inline]
    fn init(&self) -> Arc<T> {
        let service = self
            .service
            .upgrade()
            .expect("services map exists for dependency initialization.");

        require::<T>(&service, self.name)
    }
}

impl<'a> Args<'a> {
    /// Create a lazy-reference to a service when constructing another Service.
    #[inline]
    pub fn depend<T: Service>(&'a self, name: &'static str) -> Dep<T> {
        Dep::<T> {
            dep: OnceLock::new(),
            service: Arc::downgrade(self.service),
            name,
        }
    }

    /// Create a reference immediately to a service when constructing another
    /// Service. The other service must be constructed.
    #[inline]
    pub fn require<T: Service>(&'a self, name: &str) -> Arc<T> {
        require::<T>(self.service, name)
    }
}

/// Record a constructed Service under its own name, so that the services built
/// after it can reference it.
///
/// Both handles are weak: the map is what services look each other up through,
/// not what keeps them alive. The caller owns the strong references.
pub fn add(map: &Map, service: Arc<dyn Service>, any: Arc<dyn Any + Send + Sync>) {
    let name = service.name().to_owned();
    let mut map = map.write().expect("locked for writing");

    trace!("built service #{}: {name:?}", map.len());

    map.insert(name, (Arc::downgrade(&service), Arc::downgrade(&any)));
}

/// Reference a Service by name. Panics if the Service does not exist or was
/// incorrectly cast.
#[inline]
fn require<T: Service>(map: &Map, name: &str) -> Arc<T> {
    try_get::<T>(map, name)
        .inspect_err(inspect_log)
        .expect("Failed to reference service required by another service.")
}

/// Reference a Service by name. Returns None if the Service does not exist, but
/// panics if incorrectly cast.
///
/// # Panics
/// Incorrect type is not a silent failure (None) as the type never has a reason
/// to be incorrect.
pub fn get<T>(map: &Map, name: &str) -> Option<Arc<T>>
where
    T: Any + Send + Sync + Sized,
{
    map.read()
        .expect("locked for reading")
        .get(name)
        .map(|(_, s)| {
            s.upgrade().map(|s| {
                s.downcast::<T>()
                    .expect("Service must be correctly downcast.")
            })
        })?
}

/// Reference a Service by name. Returns Err if the Service does not exist or
/// was incorrectly cast.
pub fn try_get<T>(map: &Map, name: &str) -> Result<Arc<T>>
where
    T: Any + Send + Sync + Sized,
{
    map.read()
        .expect("locked for reading")
        .get(name)
        .map_or_else(
            || Err!("Service {name:?} does not exist or has not been built yet."),
            |(_, s)| {
                s.upgrade().map_or_else(
                    || Err!("Service {name:?} no longer exists."),
                    |s| {
                        s.downcast::<T>()
                            .map_err(|_| err!("Service {name:?} must be correctly downcast."))
                    },
                )
            },
        )
}

/// Utility for service implementations; see Service::name() in the trait.
#[inline]
pub fn make_name(module_path: &str) -> &str {
    module_path.split_once_infallible("::").1
}
