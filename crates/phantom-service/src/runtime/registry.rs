use std::{
    any::{Any, TypeId, type_name},
    collections::BTreeMap,
    ops::Deref,
    sync::{Arc, OnceLock, RwLock, Weak},
};

use phantom_core::{
    Err, Result, diagnostics::error::inspect_log, err, text::SplitInfallible, trace,
};

use super::contract::Service;

pub type Map = RwLock<MapType>;
pub type MapType = BTreeMap<MapKey, MapVal>;
pub type MapVal = (Weak<dyn Service>, Weak<dyn Any + Send + Sync>, TypeId);
pub type MapKey = String;

/// A lazily resolved reference to another service.
///
/// The service is found by its type rather than by a name string, so a
/// dependency cannot name the wrong service. `Services::build` checks every
/// `Dep` requested during the build resolves before the server starts, which
/// is a deviation from tuwunel, where a missing service panics at first use.
pub struct Dep<T: Service> {
    dep: OnceLock<Arc<T>>,
    service: Weak<Map>,
}

impl<T: Service> Dep<T> {
    #[inline]
    pub(super) fn new(service: &Arc<Map>) -> Self {
        Self {
            dep: OnceLock::new(),
            service: Arc::downgrade(service),
        }
    }

    #[inline]
    fn init(&self) -> Arc<T> {
        let service = self
            .service
            .upgrade()
            .expect("services map exists for dependency initialization.");

        require::<T>(&service)
    }
}

impl<T: Service> Deref for Dep<T> {
    type Target = Arc<T>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.dep.get_or_init(
            #[inline(never)]
            || self.init(),
        )
    }
}

pub fn add(map: &Map, service: Arc<dyn Service>, any: Arc<dyn Any + Send + Sync>) {
    let name = service.name().to_owned();
    let type_id = Any::type_id(&*any);
    let mut map = map.write().expect("locked for writing");

    trace!("built service #{}: {name:?}", map.len());

    map.insert(
        name,
        (Arc::downgrade(&service), Arc::downgrade(&any), type_id),
    );
}

#[inline]
pub(super) fn require<T: Service>(map: &Map) -> Arc<T> {
    try_get::<T>(map)
        .inspect_err(inspect_log)
        .expect("Failed to reference service required by another service.")
}

/// Whether a service of the given type has been built into `map`.
pub(super) fn contains(map: &Map, type_id: TypeId) -> bool {
    map.read()
        .expect("locked for reading")
        .values()
        .any(|&(.., id)| id == type_id)
}

pub fn get<T>(map: &Map) -> Option<Arc<T>>
where
    T: Any + Send + Sync + Sized,
{
    try_get::<T>(map).ok()
}

pub fn try_get<T>(map: &Map) -> Result<Arc<T>>
where
    T: Any + Send + Sync + Sized,
{
    let name = service_name(type_name::<T>());

    map.read()
        .expect("locked for reading")
        .values()
        .find(|&&(.., id)| id == TypeId::of::<T>())
        .map_or_else(
            || Err!("Service {name:?} does not exist or has not been built yet."),
            |(_, s, _)| {
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

/// A service's registry-style name from its type name, for diagnostics:
/// `phantom_service::accounts::users::Service` becomes `accounts::users`.
pub(super) fn service_name(type_name: &str) -> &str {
    let path = make_name(type_name);

    path.strip_suffix("::Service").unwrap_or(path)
}

#[inline]
pub fn make_name(module_path: &str) -> &str {
    module_path.split_once_infallible("::").1
}
