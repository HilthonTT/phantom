pub mod provider;

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use derive_more::Debug;
use futures::TryStreamExt;
pub use object_store::{CopyMode, GetResult, GetResultPayload, PutPayload, PutResult};
use phantom_core::{
    Result, at,
    config::{StorageProvider, StorageProviderLocal},
    err, implement,
    stream::IterStream,
};

pub use self::provider::Provider;
use crate::{Dep, ops::config};

#[derive(Debug)]
pub struct Service {
    providers: Providers,

    #[debug(skip)]
    services: Services,
}

struct Services {
    config: Dep<config::Service>,
}

type Providers = BTreeMap<String, Arc<Provider>>;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            providers: build_providers(&args)?,
            services: Services {
                config: args.depend::<config::Service>("ops::config"),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        self.start_providers().await?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[tracing::instrument(level = "info", err(level = "error"), skip_all)]
fn build_providers(args: &crate::Args<'_>) -> Result<Providers> {
    let config = &args.server.config;

    let default_media_provider = (!config.storage_provider.contains_key("media")).then(|| {
        let provider = StorageProviderLocal {
            create_if_missing: true,
            base_path: config.media_path().to_string_lossy().into(),

            ..Default::default()
        };

        ("media".to_owned(), StorageProvider::local(provider))
    });

    config
        .storage_provider
        .iter()
        .chain(
            default_media_provider
                .iter()
                .map(|(name, conf)| (name, conf)),
        )
        .filter_map(|(name, conf)| match conf {
            StorageProvider::local(conf) => provider::local::new(args, name, conf).transpose(),
            StorageProvider::s3(conf) => provider::s3::new(args, name, conf).transpose(),
            StorageProvider::None => None,
        })
        .collect::<Result<_>>()
}

#[implement(Service)]
async fn start_providers(&self) -> Result {
    self.providers
        .iter()
        .map(at!(1))
        .try_stream()
        .and_then(Provider::start)
        .try_collect()
        .await
}

/// Get the specific storage provider's instance by ID.
#[implement(Service)]
pub fn provider<'a>(&'a self, id: &'a str) -> Result<&'a Arc<Provider>> {
    self.providers
        .get(id)
        .ok_or_else(|| err!(Request(NotFound("No instance of provider"))))
}

/// Get the specific storage provider's configuration by ID.
#[implement(Service)]
pub fn config<'a>(&'a self, id: &'a str) -> Result<&'a StorageProvider> {
    self.configs(Some(id))
        .next()
        .map(at!(1))
        .ok_or_else(|| err!(Request(NotFound("No configuration for provider"))))
}

/// Iterate the storage provider instances.
#[implement(Service)]
pub fn providers(&self) -> impl Iterator<Item = &Arc<Provider>> + Send + '_ {
    self.providers.values()
}

/// Iterate the storage provider configurations.
#[implement(Service)]
pub fn configs<'a, Id>(
    &'a self,
    id: Id,
) -> impl Iterator<Item = (&'a String, &'a StorageProvider)> + Send + 'a
where
    Id: Into<Option<&'a str>>,
{
    let id = id.into();

    self.services
        .config
        .storage_provider
        .iter()
        .filter(move |(id_, _)| id.is_none_or(|id| id_.starts_with(id)))
}
