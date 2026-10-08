pub(super) mod legacy;

use std::{str::FromStr, time::Duration};

use axum::extract::State;
use phantom_core::{
    Err, Result, content_disposition::make_content_disposition, err, math::ruma_from_usize, rand,
};
use phantom_service::{
    Services,
    media::{Dim, Dimensions, FileMeta, MXC_LENGTH},
};
use ruma::{
    MilliSecondsSinceUnixEpoch, MxcUri, OwnedMxcUri, UInt, UserId,
    api::{
        IncomingResponse,
        client::{
            authenticated_media::{
                get_content, get_content_as_filename, get_content_thumbnail, get_media_config,
                get_media_preview,
            },
            media::{create_content, create_content_async, create_mxc_uri},
        },
    },
    http_headers::ContentDisposition,
};
use serde_json::value::RawValue as RawJsonValue;
use url::Url;

use crate::router::{ClientIp, Ruma};

/// Served when the stored media carries no type of its own.
const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

/// A stored file with the headers it is served under.
pub(super) struct Media {
    pub(super) content: Vec<u8>,
    pub(super) content_type: String,
    pub(super) content_disposition: ContentDisposition,
}

/// # `GET /_matrix/client/v1/media/config`
pub(crate) async fn get_media_config_route(
    State(services): State<crate::router::State>,
    _body: Ruma<get_media_config::v1::Request>,
) -> Result<get_media_config::v1::Response> {
    Ok(get_media_config::v1::Response::new(ruma_from_usize(
        services.server.config.network.max_request_size,
    )))
}

/// # `POST /_matrix/media/v3/upload`
///
/// Permanently save media in the server.
///
/// - Some metadata will be saved in the database
/// - Media will be saved in the media/ directory
#[tracing::instrument(
    name = "media_upload",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn create_content_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<create_content::v3::Request>,
) -> Result<create_content::v3::Response> {
    let user = body.sender_user();

    let filename = body.filename.as_deref();
    let content_type = body.content_type.as_deref();
    let content_disposition = make_content_disposition(None, content_type, filename);
    let mxc = new_mxc(&services);

    services
        .media
        .create(
            &mxc,
            Some(user),
            Some(&content_disposition),
            content_type,
            &body.file,
        )
        .await?;

    Ok(create_content::v3::Response::new(mxc))
}

/// # `POST /_matrix/media/v1/create`
///
/// Create a new MXC URI without content.
#[tracing::instrument(
    name = "media_create_mxc",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn create_mxc_uri_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<create_mxc_uri::v1::Request>,
) -> Result<create_mxc_uri::v1::Response> {
    let user = body.sender_user();
    let mxc = new_mxc(&services);

    let unused_expires_at = services.media.create_pending(&mxc, user).await?;

    let mut response = create_mxc_uri::v1::Response::new(mxc);
    response.unused_expires_at = UInt::new(unused_expires_at).map(MilliSecondsSinceUnixEpoch);

    Ok(response)
}

/// # `PUT /_matrix/media/v3/upload/{serverName}/{mediaId}`
///
/// Upload content to a MXC URI that was created earlier.
#[tracing::instrument(
    name = "media_upload_async",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn create_content_async_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<create_content_async::v3::Request>,
) -> Result<create_content_async::v3::Response> {
    let user = body.sender_user();
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let filename = body.filename.as_deref();
    let content_type = body.content_type.as_deref();
    let content_disposition = make_content_disposition(None, content_type, filename);

    services
        .media
        .upload_pending(
            &mxc,
            user,
            Some(&content_disposition),
            content_type,
            &body.file,
        )
        .await?;

    // ruma 0.17 gives this empty, non-exhaustive response no constructor, so
    // it is decoded from the body it would be sent as.
    create_content_async::v3::Response::try_from_http_response_inner(http::Response::new(b"{}"))
        .map_err(|e| err!("Could not build the upload response: {e}"))
}

/// # `GET /_matrix/client/v1/media/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
#[tracing::instrument(
    name = "media_thumbnail_get",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn get_content_thumbnail_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content_thumbnail::v1::Request>,
) -> Result<get_content_thumbnail::v1::Response> {
    let dim = Dim::from_ruma(body.width, body.height, body.method.clone())?;
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let Media {
        content,
        content_type,
        content_disposition,
    } = fetch_thumbnail(&services, &mxc, body.timeout_ms, &dim).await?;

    Ok(get_content_thumbnail::v1::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// # `GET /_matrix/client/v1/media/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
#[tracing::instrument(
    name = "media_get",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn get_content_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content::v1::Request>,
) -> Result<get_content::v1::Response> {
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let Media {
        content,
        content_type,
        content_disposition,
    } = fetch_file(&services, &mxc, body.timeout_ms, None).await?;

    Ok(get_content::v1::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// # `GET /_matrix/client/v1/media/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation as fileName.
#[tracing::instrument(
    name = "media_get_af",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn get_content_as_filename_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content_as_filename::v1::Request>,
) -> Result<get_content_as_filename::v1::Response> {
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let Media {
        content,
        content_type,
        content_disposition,
    } = fetch_file(&services, &mxc, body.timeout_ms, Some(&body.filename)).await?;

    Ok(get_content_as_filename::v1::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// # `GET /_matrix/client/v1/media/preview_url`
///
/// Returns URL preview.
#[tracing::instrument(
    name = "url_preview",
    level = "debug",
    skip_all,
    fields(%client),
)]
pub(crate) async fn get_media_preview_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_media_preview::v1::Request>,
) -> Result<get_media_preview::v1::Response> {
    url_preview(&services, body.sender_user(), &body.url)
        .await
        .map(get_media_preview::v1::Response::from_raw_value)
}

/// Fetches the preview of `url` for the authenticated and legacy routes.
pub(super) async fn url_preview(
    services: &Services,
    sender_user: &UserId,
    url: &str,
) -> Result<Box<RawJsonValue>> {
    let url = Url::parse(url).map_err(|e| {
        err!(Request(InvalidParam(debug_warn!(
            "Requested URL {url} from {sender_user} is not valid: {e}"
        ))))
    })?;

    if !services.media.url_preview_allowed(&url) {
        return Err!(Request(Forbidden(debug_warn!(
            "URL {url} from {sender_user} is not allowed to be previewed"
        ))));
    }

    let preview = services
        .media
        .get_url_preview(&url)
        .await
        .map_err(|error| {
            err!(Request(Unknown(debug_error!(
                "Failed to fetch URL preview of {url} for {sender_user}: {error}"
            ))))
        })?;

    serde_json::value::to_raw_value(&preview).map_err(|error| {
        err!(Request(Unknown(debug_error!(
            "Failed to parse URL preview of {url} for {sender_user}: {error}"
        ))))
    })
}

/// Loads a thumbnail, waiting up to `timeout` for a pending upload to land.
pub(super) async fn fetch_thumbnail(
    services: &Services,
    mxc: &MxcUri,
    timeout: Duration,
    dim: &Dim,
) -> Result<Media> {
    let (meta, content) = match services.media.get_or_fetch_thumbnail(mxc, dim).await {
        Ok(found) => found,
        Err(e) => {
            if services.media.await_pending(mxc, timeout).await.is_err() {
                return Err(e);
            }

            services.media.get_thumbnail(mxc, dim).await?
        }
    };

    Ok(media(meta, content, None))
}

/// Loads a file, waiting up to `timeout` for a pending upload to land.
pub(super) async fn fetch_file(
    services: &Services,
    mxc: &MxcUri,
    timeout: Duration,
    filename: Option<&str>,
) -> Result<Media> {
    let (meta, content) = match services.media.get_or_fetch(mxc).await {
        Ok(found) => found,
        Err(e) => {
            if services.media.await_pending(mxc, timeout).await.is_err() {
                return Err(e);
            }

            services.media.get(mxc, Dimensions::ORIGINAL).await?
        }
    };

    Ok(media(meta, content, filename))
}

/// Recomputes the served disposition against the stored type, so stored
/// headers can't make the browser render untrusted content inline.
pub(super) fn media(meta: FileMeta, content: Vec<u8>, filename: Option<&str>) -> Media {
    let stored = meta
        .content_disposition
        .as_deref()
        .and_then(|value| ContentDisposition::from_str(value).ok());

    let content_disposition =
        make_content_disposition(stored.as_ref(), meta.content_type.as_deref(), filename);

    Media {
        content,
        content_type: meta
            .content_type
            .unwrap_or_else(|| DEFAULT_CONTENT_TYPE.to_owned()),
        content_disposition,
    }
}

pub(super) fn mxc_of(server_name: &ruma::ServerName, media_id: &str) -> OwnedMxcUri {
    OwnedMxcUri::from(format!("mxc://{server_name}/{media_id}"))
}

fn new_mxc(services: &Services) -> OwnedMxcUri {
    mxc_of(
        services.server_state.server_name(),
        &rand::string(MXC_LENGTH),
    )
}
