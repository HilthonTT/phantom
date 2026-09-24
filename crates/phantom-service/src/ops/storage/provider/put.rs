//! Storing an object, single-part or multi-part.

use std::iter::from_fn;

use bytes::Bytes;
use futures::{Stream, TryFutureExt, TryStreamExt};
use object_store::{MultipartUpload, ObjectStoreExt, PutPayload, PutResult};
use phantom_core::{
    Error, Result, config::StorageProvider, debug, error, extract_variant, implement,
    result::FlatOk, stream::IterStream,
};

use super::Provider;

/// Put object into store from streaming input.
///
/// Recommended to know the total size of the object. If size is `None`,
/// multi-part upload may be selected even for small uploads below the
/// configured threshold.
#[implement(Provider)]
#[tracing::instrument(
	level = "debug",
	err(level = "debug"),
	skip_all,
	fields(
		provider = %self.name,
		?path,
		?size,
	)
)]
pub async fn put<S, T>(&self, path: &str, size: Option<usize>, input: S) -> Result<PutResult>
where
    S: Stream<Item = Result<T>> + Send,
    PutPayload: From<T> + From<PutPayload>,
{
    if size.is_none_or(|size| size >= self.multipart_threshold()) {
        return self.put_multi(path, input).await;
    }

    debug!(
        ?size,
        threshold = ?self.multipart_threshold(),
        "Selecting single-part upload..."
    );

    let payload: PutPayload = input
        .map_ok(PutPayload::from)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(Bytes::from)
        .collect();

    self.put_single(path, payload).await
}

/// Put object into the store from contiguous input.
///
/// The size of input will be determined and multipart upload will be chosen as
/// necessary internally.
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
pub async fn put_one<T>(&self, path: &str, input: T) -> Result<PutResult>
where
    PutPayload: From<T> + From<PutPayload>,
{
    let payload: PutPayload = input.into();

    if payload.content_length() < self.multipart_threshold() {
        return self.put_single(path, payload).await;
    }

    let part_size = self.multipart_part_size();

    debug!(
        len = ?payload.content_length(),
        threshold = ?self.multipart_threshold(),
        ?part_size,
        "Selecting multi-part upload..."
    );

    self.put_multi(path, chunked(payload, part_size).try_stream())
        .await
}

/// Put object into the store from streaming input using multipart upload.
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
async fn put_multi<S, T>(&self, path: &str, input: S) -> Result<PutResult>
where
    S: Stream<Item = Result<T>> + Send,
    PutPayload: From<T> + From<PutPayload>,
{
    let path = self.to_abs_path(path)?;
    let mut handle = self
        .provider
        .put_multipart(&path)
        .map_err(Error::from)
        .await?;

    match input
        .try_for_each(|t| handle.put_part(t.into()).map_err(Error::from))
        .inspect_err(|e| error!(?path, "Failed to store object: {e}"))
        .await
    {
        Ok(()) => {
            handle
                .complete()
                .map_err(Error::from)
                .inspect_err(|e| {
                    error!(?path, "Failed to store object during completion: {e}");
                })
                .await
        }

        Err(e) => {
            handle
                .abort()
                .map_ok(|()| Err(e))
                .map_err(Error::from)
                .inspect_err(|e| {
                    error!(?path, "Additional errors during error handling: {e}");
                })
                .await?
        }
    }
}

/// Put object into the store from contiguous input non-multipart upload.
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
async fn put_single(&self, path: &str, input: PutPayload) -> Result<PutResult> {
    let path = self.to_abs_path(path)?;

    self.provider.put(&path, input).map_err(Error::from).await
}

#[implement(Provider)]
fn multipart_threshold(&self) -> usize {
    extract_variant!(&self.config, StorageProvider::s3)
        .map(|config| config.multipart_threshold.as_u64())
        .map(TryInto::try_into)
        .flat_ok()
        .unwrap_or(usize::MAX)
}

#[implement(Provider)]
fn multipart_part_size(&self) -> usize {
    extract_variant!(&self.config, StorageProvider::s3)
        .map(|config| config.multipart_part_size.as_u64())
        .map(TryInto::try_into)
        .flat_ok()
        .unwrap_or(usize::MAX)
}

pub(super) fn chunked(payload: PutPayload, part_size: usize) -> impl Iterator<Item = PutPayload> {
    let mut buf: Bytes = payload.into();

    from_fn(move || (!buf.is_empty()).then(|| buf.split_to(part_size.min(buf.len())).into()))
}
