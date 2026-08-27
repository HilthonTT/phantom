//! The map every built service is recorded in, and how one reaches another.
//!
//! Services hold each other through [`Dep`] rather than directly: two of them
//! may depend on each other, and a pair of `Arc`s pointing both ways is a
//! cycle that never drops. The map keeps only weak references — the strong
//! ones belong to [`super::Services`] — so a lookup can find a service without
//! keeping it alive past shutdown.

use std::{
    any::Any,
    collections::BTreeMap,
    ops::Deref,
    sync::{Arc, OnceLock, RwLock, Weak},
};

use phantom_core::{Err, Result, err, error::inspect_log, text::SplitInfallible, trace};

use super::contract::Service;

pub type Map = RwLock<MapType>;
pub type MapType = BTreeMap<MapKey, MapVal>;
pub type MapVal = (Weak<dyn Service>, Weak<dyn Any + Send + Sync>);
pub type MapKey = String;

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

impl<T: Service + Send + Sync> Dep<T> {
    /// A lazy handle on the service registered under `name`. Nothing is looked
    /// up until the handle is first dereferenced, which is what lets a service
    /// take one on a service built after it.
    #[inline]
    pub(super) fn new(service: &Arc<Map>, name: &'static str) -> Self {
        Self {
            dep: OnceLock::new(),
            service: Arc::downgrade(service),
            name,
        }
    }

    #[inline]
    fn init(&self) -> Arc<T> {
        let service = self
            .service
            .upgrade()
            .expect("services map exists for dependency initialization.");

        require::<T>(&service, self.name)
    }
}

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
pub(super) fn require<T: Service>(map: &Map, name: &str) -> Arc<T> {
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

/// A service's name: its module path with the crate name stripped off, e.g.
/// `resolver` for `phantom_service::resolver`. See [`Service::name`].
#[inline]
pub fn make_name(module_path: &str) -> &str {
    module_path.split_once_infallible("::").1
}
