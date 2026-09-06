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
    MxcUri,
    api::federation::authenticated_media::{FileOrLocation, get_content},
    http_headers::ContentDisposition,
};

use super::{Dimensions, FileMeta, Service, parts};
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

    if self
        .services
        .moderation
        .forbids(server_name, Restriction::Media)
    {
        return Err!(Request(Forbidden(
            "This server does not download media from {server_name}."
        )));
    }

    let request = get_content::v1::Request::new(media_id.to_owned());

    let response = self
        .services
        .federation
        .execute(server_name, request)
        .await?;

    let content = match response.content {
        FileOrLocation::File(content) => content,
        // A server may answer with a URL instead of the bytes. Following it
        // would be this server making an arbitrary outbound request on a
        // remote server's say-so, which is a different and much larger trust
        // decision than federating with it.
        FileOrLocation::Location(location) => {
            return Err!(BadServerResponse(
                "{server_name} redirected media {mxc} to {location}, which is not followed."
            ));
        }
        // The enum is non-exhaustive because the spec may grow another way of
        // answering. Anything we do not recognize is not a file we can serve.
        _ => {
            return Err!(BadServerResponse(
                "{server_name} answered for media {mxc} in a form this server does not understand."
            ));
        }
    };

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

    self.create(
        mxc,
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
