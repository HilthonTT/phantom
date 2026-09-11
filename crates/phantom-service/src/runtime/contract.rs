use std::{any::Any, fmt::Write, sync::Arc};

use async_trait::async_trait;
use phantom_core::{Result, server::Server};
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
}

impl<'a> Args<'a> {
    #[inline]
    pub fn depend<T: Service>(&'a self, name: &'static str) -> Dep<T> {
        Dep::<T>::new(self.service, name)
    }

    #[inline]
    pub fn require<T: Service>(&'a self, name: &str) -> Arc<T> {
        require::<T>(self.service, name)
    }
}
