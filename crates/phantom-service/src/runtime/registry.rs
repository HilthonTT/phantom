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

pub struct Dep<T: Service> {
    dep: OnceLock<Arc<T>>,
    service: Weak<Map>,
    name: &'static str,
}

impl<T: Service> Dep<T> {
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
    let mut map = map.write().expect("locked for writing");

    trace!("built service #{}: {name:?}", map.len());

    map.insert(name, (Arc::downgrade(&service), Arc::downgrade(&any)));
}

#[inline]
pub(super) fn require<T: Service>(map: &Map, name: &str) -> Arc<T> {
    try_get::<T>(map, name)
        .inspect_err(inspect_log)
        .expect("Failed to reference service required by another service.")
}

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

#[inline]
pub fn make_name(module_path: &str) -> &str {
    module_path.split_once_infallible("::").1
}
