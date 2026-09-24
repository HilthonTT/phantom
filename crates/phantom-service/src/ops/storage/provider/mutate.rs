//! Deleting, renaming and copying objects already in the store.

use std::{iter::once, sync::Arc};

use futures::{FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt};
use object_store::{CopyMode, ObjectStore, ObjectStoreExt, path::Path};
use phantom_core::{
    Error, Result, implement,
    stream::{IterStream, TryReadyExt},
};

use super::Provider;

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "debug"),
	skip_all,
	fields(
		provider = %self.name,
		?path,
	)
)]
pub async fn delete_one(self: &Arc<Self>, path: &str) -> Result {
    self.delete(once(path.to_owned()).stream())
        .map_ok(|_| ())
        .try_collect()
        .await
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		provider = %self.name,
	)
)]
pub fn delete<S>(self: &Arc<Self>, paths: S) -> impl Stream<Item = Result<Path>> + Send
where
    S: Stream<Item = String> + Send + 'static,
{
    let this = self.clone();
    let paths = paths
        .map(Ok)
        .ready_and_then(move |path| {
            use object_store::{Error, path};

            this.to_abs_path(&path).map_err(|_| Error::InvalidPath {
                source: path::Error::InvalidPath { path: path.into() },
            })
        })
        .boxed();

    self.provider.delete_stream(paths).map_err(Error::from)
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "debug"),
	skip_all,
	fields(
		provider = %self.name,
		?src,
		?dst,
		?overwrite,
	)
)]
pub async fn rename(&self, src: &str, dst: &str, overwrite: CopyMode) -> Result {
    let src = self.to_abs_path(src)?;
    let dst = self.to_abs_path(dst)?;

    match overwrite {
        CopyMode::Overwrite => self.provider.rename(&src, &dst).left_future(),
        CopyMode::Create => self
            .provider
            .rename_if_not_exists(&src, &dst)
            .right_future(),
    }
    .map_err(Error::from)
    .await
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "debug"),
	skip_all,
	fields(
		provider = %self.name,
		?src,
		?dst,
		?overwrite,
	)
)]
pub async fn copy(&self, src: &str, dst: &str, overwrite: CopyMode) -> Result {
    let src = self.to_abs_path(src)?;
    let dst = self.to_abs_path(dst)?;

    match overwrite {
        CopyMode::Overwrite => self.provider.copy(&src, &dst).left_future(),
        CopyMode::Create => self.provider.copy_if_not_exists(&src, &dst).right_future(),
    }
    .map_err(Error::from)
    .await
}
