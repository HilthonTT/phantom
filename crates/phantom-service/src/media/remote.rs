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
#[implement(Service)]
pub async fn get_or_fetch(&self, mxc: &MxcUri) -> Result<(FileMeta, Vec<u8>)> {
    if let Ok(found) = self.get(mxc, Dimensions::ORIGINAL).await {
        return Ok(found);
    }

    let (server_name, _) = parts(mxc)?;

    if self.services.server_state.server_is_ours(server_name) {
        return Err!(Request(NotFound("Media {mxc} is not stored here.")));
    }

    self.fetch(mxc).await
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
