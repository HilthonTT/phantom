//! Reading an object, its metadata, and presigned URLs for it.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use futures::{Stream, TryFutureExt, TryStreamExt};
use http::Method;
use object_store::{GetResult, ObjectMeta, ObjectStore, ObjectStoreExt, path::Path};
use phantom_core::{Error, Result, implement};
use url::Url;

use super::{FetchItem, FetchMetaItem, Provider};

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		provider = %self.name,
		?path,
	)
)]
pub fn fetch_with_metadata(&self, path: &str) -> impl Stream<Item = Result<FetchMetaItem>> + Send {
    self.load(path)
        .map_ok(|result| {
            let meta = (
                result.range.clone(),
                result.meta.clone(),
                result.attributes.clone(),
            );
            let data = Arc::new(meta);

            result
                .into_stream()
                .map_err(Error::from)
                .map_ok(move |bytes| (bytes, data.clone()))
        })
        .map_err(Error::from)
        .try_flatten_stream()
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		provider = %self.name,
		?path,
	)
)]
pub fn fetch(&self, path: &str) -> impl Stream<Item = Result<FetchItem>> + Send {
    self.load(path)
        .map_ok(|result| {
            let size = result.meta.size;
            let range = result.range.clone();

            result
                .into_stream()
                .map_err(Error::from)
                .map_ok(move |bytes| (bytes, (range.clone(), size)))
        })
        .map_err(Error::from)
        .try_flatten_stream()
}

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
pub async fn get(&self, path: &str) -> Result<Bytes> {
    self.load(path)
        .map_ok(GetResult::bytes)
        .await?
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
		?path,
	)
)]
pub async fn load(&self, path: &str) -> Result<GetResult> {
    let path = self.to_abs_path(path)?;

    self.provider.get(&path).map_err(Error::from).await
}

/// Presign a time-limited GET URL for an object, when this provider supports
/// signing (S3).
#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "debug"),
	skip_all,
	fields(
		provider = %self.name,
		?path,
		?ttl,
	)
)]
pub async fn signed_get_url(&self, path: &str, ttl: Duration) -> Result<Option<Url>> {
    let Some(signer) = self.signer.as_ref() else {
        return Ok(None);
    };

    let path = self.to_abs_path(path)?;

    signer
        .signed_url(Method::GET, &path, ttl)
        .map_err(Error::from)
        .map_ok(Some)
        .await
}

#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	skip_all,
	fields(
		provider = %self.name,
		?prefix,
	)
)]
pub fn list(&self, prefix: Option<&str>) -> impl Stream<Item = Result<ObjectMeta>> + Send {
    let abs_prefix = prefix
        .map(Path::from)
        .map(|p| self.prepend_base_path(p))
        .or_else(|| self.base_path.clone());

    self.provider
        .list(abs_prefix.as_ref())
        .map_err(Error::from)
        .map_ok(|meta| ObjectMeta {
            location: self.strip_base_path(meta.location),
            ..meta
        })
}

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
pub async fn head(&self, path: &str) -> Result<ObjectMeta> {
    self.provider
        .head(&self.to_abs_path(path)?)
        .map_err(Error::from)
        .await
}
