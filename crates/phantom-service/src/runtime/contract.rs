use std::{
    any::{Any, TypeId, type_name},
    fmt::Write,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use phantom_core::{Result, runtime::server::Server};
use phantom_database::Database;

use super::registry::{Dep, Map, require};

#[async_trait]
pub trait Service: Any + Send + Sync {
    fn build(args: Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized;

    async fn worker(self: Arc<Self>) -> Result<()> {
        Ok(())
    }

    fn interrupt(&self) {}

    async fn clear_cache(&self) {}

    async fn memory_usage(&self, _out: &mut (dyn Write + Send)) -> Result {
        Ok(())
    }

    fn name(&self) -> &str;

    fn unconstrained(&self) -> bool {
        false
    }
}

pub struct Args<'a> {
    pub server: &'a Arc<Server>,
    pub db: &'a Arc<Database>,
    pub service: &'a Arc<Map>,

    /// Every `Dep` taken during this build, checked once all services exist.
    pub(super) requested: &'a Mutex<Vec<Requested>>,
}

/// A dependency one service took on another while being built.
pub(super) struct Requested {
    pub(super) type_id: TypeId,
    pub(super) type_name: &'static str,
}

impl<'a> Args<'a> {
    #[inline]
    pub fn depend<T: Service>(&'a self) -> Dep<T> {
        self.requested
            .lock()
            .expect("locked for writing")
            .push(Requested {
                type_id: TypeId::of::<T>(),
                type_name: type_name::<T>(),
            });

        Dep::<T>::new(self.service)
    }

    #[inline]
    pub fn require<T: Service>(&'a self) -> Arc<T> {
        require::<T>(self.service)
    }
}
