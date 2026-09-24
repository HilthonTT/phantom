pub mod local;
mod mutate;
mod path;
mod put;
mod read;
pub mod s3;

#[cfg(test)]
mod tests;

use std::{ops::Range, sync::Arc};

use bytes::Bytes;
use derive_more::Debug;
use futures::{FutureExt, TryFutureExt, TryStreamExt};
use object_store::{Attributes, DynObjectStore, ObjectMeta, path::Path, signer::Signer};
use phantom_core::{Result, debug, error, implement, info, runtime::config::StorageProvider};

#[derive(Debug)]
pub struct Provider {
    pub name: String,

    pub config: StorageProvider,

    pub(crate) provider: Box<DynObjectStore>,

    #[debug(skip)]
    pub(crate) signer: Option<Arc<dyn Signer>>,

    pub(crate) base_path: Option<Path>,

    startup_check: bool,
}

pub type FetchItem = (Bytes, (Range<u64>, u64));
pub type FetchMetaItem = (Bytes, Arc<(Range<u64>, ObjectMeta, Attributes)>);

#[implement(Provider)]
#[tracing::instrument(skip_all, err)]
pub(super) async fn start(self: &Arc<Self>) -> Result {
    if self.startup_check {
        self.startup_check().await?;
    }

    Ok(())
}

#[implement(Provider)]
#[tracing::instrument(name = "check", skip_all, err)]
async fn startup_check(self: &Arc<Self>) -> Result {
    debug!(
        name = ?self.name,
        "Checking storage provider client connection...",
    );

    self.ping()
        .inspect_ok(|()| {
            info!(
                name = %self.name,
                "Connected to storage provider"
            );
        })
        .await
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "error"),
	skip_all,
	fields(
		provider = %self.name,
	)
)]
pub async fn ping(&self) -> Result {
    self.list(None)
        .try_next()
        .inspect_err(|e| {
            error!("Failed to connect to storage provider: {e}");
        })
        .boxed()
        .await
        .map(|_| ())
}
