//! Fetching a file from the server that holds it.
//!
//! Media on another server is served through this one rather than by
//! redirecting a client to it, which is what keeps a client's address out of
//! the hands of every server whose rooms it is in. The cost is that this
//! server downloads and stores the file, so what arrives is bounded and what
//! it came from is checked first.
//!
//! A fetched file is cached under the same media id it has at home. That is
//! what makes the cache safe to purge — `delete_from_server` removes a
//! server's files without touching anything of ours — and what makes a second
//! request for the same file free.

#[cfg(feature = "url_preview")]
use phantom_core::err;
use phantom_core::{Err, Result, debug, implement};
use ruma::{
    MxcUri, ServerName,
    api::federation::authenticated_media::{
        Content, FileOrLocation, get_content, get_content_thumbnail,
    },
    http_headers::ContentDisposition,
};

use super::{Dim, Dimensions, FileMeta, Service, parts};
use crate::moderation::Restriction;

/// Serves a file, fetching it from the server that holds it if this server
/// does not have it already.
///
/// A URI of ours that is not stored is not necessarily unknown: a URL preview
/// mints one for each piece of media a page names, without downloading it. The
/// lazy path is what resolves those, and it reports the same not-found for a
/// URI that was never minted at all.
#[implement(Service)]
pub async fn get_or_fetch(&self, mxc: &MxcUri) -> Result<(FileMeta, Vec<u8>)> {
    if let Ok(found) = self.get(mxc, Dimensions::ORIGINAL).await {
        return Ok(found);
    }

    let (server_name, _) = parts(mxc)?;

    if self.services.server_state.server_is_ours(server_name) {
        return self.fetch_lazy_media(mxc).await;
    }

    self.fetch(mxc).await
}

/// Resolves a URI a URL preview minted, on the first download of it.
///
/// A preview records the URL rather than the file, so this is where the file
/// is actually obtained — from the bytes an `og:image` measurement already
/// staged, or from the origin. Either way it is promoted into the media store
/// through the ordinary upload path, so every later download is a plain
/// media hit and the origin is contacted at most once.
///
/// One fetch per URI is in flight at a time. Without that, a URI handed to a
/// room's worth of clients at once is a room's worth of requests at the
/// origin, which is this server amplifying for whoever posted the link.
#[cfg(feature = "url_preview")]
#[implement(Service)]
#[tracing::instrument(name = "lazy", level = "debug", skip(self))]
async fn fetch_lazy_media(&self, mxc: &MxcUri) -> Result<(FileMeta, Vec<u8>)> {
    use reqwest::Url;

    let key = mxc.as_str();

    let _lock = self.federation_mutex.lock(key).await;

    // A caller that was queued behind the lock may have promoted it already.
    if let Ok(found) = self.get(mxc, Dimensions::ORIGINAL).await {
        return Ok(found);
    }

    let media = match self.db.get_lazy_content(key).await {
        Ok(media) => media,
        Err(_) => {
            let Ok(url) = self.db.search_lazy_media(key).await else {
                return Err!(Request(NotFound("Media {mxc} is not stored here.")));
            };

            let url = Url::parse(&url)
                .map_err(|e| err!(Database("Lazy media {mxc} has an unparseable URL: {e}")))?;

            self.fetch_preview_media(&url).await?
        }
    };

    self.create(
        mxc,
        None,
        media.content_disposition.as_ref(),
        media.content_type.as_deref(),
        &media.content,
    )
    .await
    .inspect_err(|_| debug!(%mxc, "Could not promote lazy media"))?;

    // The registration and its staged bytes are what made this URI resolvable
    // before it was stored; now that it is stored they are only a second copy.
    let mut txn = self.db.txn();

    self.db.remove_lazy_media(&mut txn, key);
    self.db.remove_lazy_content(&mut txn, key);

    txn.execute()?;

    let meta = FileMeta {
        content_type: media.content_type,
        content_disposition: media.content_disposition.map(|value| value.to_string()),
        size: media.content.len() as u64,
        created: super::now(),
    };

    Ok((meta, media.content))
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
async fn fetch_lazy_media(&self, mxc: &MxcUri) -> Result<(FileMeta, Vec<u8>)> {
    Err!(Request(NotFound("Media {mxc} is not stored here.")))
}

/// Downloads a file from the server that holds it and stores it.
#[implement(Service)]
#[tracing::instrument(name = "fetch", level = "debug", skip(self))]
pub async fn fetch(&self, mxc: &MxcUri) -> Result<(FileMeta, Vec<u8>)> {
    let (server_name, media_id) = parts(mxc)?;

    self.may_fetch_from(server_name)?;

    let request = get_content::v1::Request::new(media_id.to_owned());

    let response = self
        .services
        .federation
        .execute(server_name, request)
        .await?;

    let content = file_of(mxc, server_name, response.content)?;

    self.store_fetched(mxc, Dimensions::ORIGINAL, content).await
}

/// Downloads a thumbnail from the server that holds the media, and stores it.
///
/// Asked for at the size the request was rounded to, and stored under that
/// size whatever the origin actually sent: a server is entitled to answer
/// with a picture of its own choosing, and a client displays a thumbnail
/// rather than measuring it. Storing it under the requested size is what
/// makes the next request for that size a local read.
#[implement(Service)]
#[tracing::instrument(name = "fetch_thumbnail", level = "debug", skip(self))]
pub(super) async fn fetch_thumbnail(&self, mxc: &MxcUri, dim: &Dim) -> Result<(FileMeta, Vec<u8>)> {
    let (server_name, media_id) = parts(mxc)?;

    self.may_fetch_from(server_name)?;

    let mut request = get_content_thumbnail::v1::Request::new(
        media_id.to_owned(),
        dim.width.into(),
        dim.height.into(),
    );

    request.method = Some(dim.method.clone());

    // An animated thumbnail is worth having for the media that has one, and
    // a server that cannot make one answers with a still.
    request.animated = Some(true);

    let response = self
        .services
        .federation
        .execute(server_name, request)
        .await?;

    let content = file_of(mxc, server_name, response.content)?;

    self.store_fetched(mxc, dim.dimensions(), content).await
}

/// Whether this server downloads media from that one at all.
#[implement(Service)]
fn may_fetch_from(&self, server_name: &ServerName) -> Result {
    if self
        .services
        .moderation
        .forbids(server_name, Restriction::Media)
    {
        return Err!(Request(Forbidden(
            "This server does not download media from {server_name}."
        )));
    }

    Ok(())
}

/// The bytes out of a federation media response.
///
/// A server may answer with a URL instead of the file. Following it would be
/// this server making an arbitrary outbound request on a remote server's say
/// so, which is a different and much larger trust decision than federating
/// with it, so it is refused.
fn file_of(mxc: &MxcUri, server_name: &ServerName, content: FileOrLocation) -> Result<Content> {
    match content {
        FileOrLocation::File(content) => Ok(content),
        FileOrLocation::Location(location) => Err!(BadServerResponse(
            "{server_name} redirected media {mxc} to {location}, which is not followed."
        )),
        // The enum is non-exhaustive because the spec may grow another way of
        // answering. Anything we do not recognize is not a file we can serve.
        _ => Err!(BadServerResponse(
            "{server_name} answered for media {mxc} in a form this server does not understand."
        )),
    }
}

/// Stores what a fetch came back with, and reports it as a download serves it.
#[implement(Service)]
async fn store_fetched(
    &self,
    mxc: &MxcUri,
    dimensions: Dimensions,
    content: Content,
) -> Result<(FileMeta, Vec<u8>)> {
    let content_disposition =
        ContentDisposition::new(phantom_core::content_disposition::content_disposition_type(
            content.content_type.as_deref(),
        ))
        .with_filename(
            content
                .content_disposition
                .as_ref()
                .and_then(|disposition| disposition.filename.as_deref())
                .map(phantom_core::content_disposition::sanitise_filename),
        );

    self.create_at(
        mxc,
        dimensions,
        None,
        Some(&content_disposition),
        content.content_type.as_deref(),
        &content.file,
    )
    .await?;

    debug!(%mxc, size = content.file.len(), "Fetched remote media");

    let meta = FileMeta {
        content_type: content.content_type,
        content_disposition: Some(content_disposition.to_string()),
        size: content.file.len() as u64,
        created: super::now(),
    };

    Ok((meta, content.file))
}
