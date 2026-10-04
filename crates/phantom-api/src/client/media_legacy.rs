#![expect(deprecated)]

use axum::extract::State;
use phantom_core::{Result, math::ruma_from_usize};
use phantom_service::media::Dim;
use ruma::{
    MxcUri,
    api::client::media::{
        get_content, get_content_as_filename, get_content_thumbnail, get_media_config,
        get_media_preview,
    },
};

use super::media::{Media, fetch_file, fetch_thumbnail, media, mxc_of, url_preview};
use crate::router::{ClientIp, Ruma};

/// # `GET /_matrix/media/v3/config`
///
/// Returns max upload size.
pub(crate) async fn get_media_config_legacy_route(
    State(services): State<crate::router::State>,
    _body: Ruma<get_media_config::v3::Request>,
) -> Result<get_media_config::v3::Response> {
    Ok(get_media_config::v3::Response::new(ruma_from_usize(
        services.server.config.network.max_request_size,
    )))
}

/// # `GET /_matrix/media/v3/preview_url`
///
/// Returns URL preview.
#[tracing::instrument(skip_all, fields(%client), name = "url_preview_legacy", level = "debug")]
pub(crate) async fn get_media_preview_legacy_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_media_preview::v3::Request>,
) -> Result<get_media_preview::v3::Response> {
    url_preview(&services, body.sender_user(), &body.url)
        .await
        .map(get_media_preview::v3::Response::from_raw_value)
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}`
///
/// Load media from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get_legacy", level = "debug")]
pub(crate) async fn get_content_legacy_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content::v3::Request>,
) -> Result<get_content::v3::Response> {
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let Media {
        content,
        content_type,
        content_disposition,
    } = fetch_legacy(&services, &mxc, body.allow_remote, body.timeout_ms, None).await?;

    Ok(get_content::v3::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// # `GET /_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}`
///
/// Load media from our server or over federation, permitting desired filename.
///
/// - Only allows federation if `allow_remote` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_get_legacy", level = "debug")]
pub(crate) async fn get_content_as_filename_legacy_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content_as_filename::v3::Request>,
) -> Result<get_content_as_filename::v3::Response> {
    let mxc = mxc_of(&body.server_name, &body.media_id);

    let Media {
        content,
        content_type,
        content_disposition,
    } = fetch_legacy(
        &services,
        &mxc,
        body.allow_remote,
        body.timeout_ms,
        Some(&body.filename),
    )
    .await?;

    Ok(get_content_as_filename::v3::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// # `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`
///
/// Load media thumbnail from our server or over federation.
///
/// - Only allows federation if `allow_remote` is true
/// - Uses client-provided `timeout_ms` if available, else defaults to 20
///   seconds
#[tracing::instrument(skip_all, fields(%client), name = "media_thumbnail_get_legacy", level = "debug")]
pub(crate) async fn get_content_thumbnail_legacy_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content_thumbnail::v3::Request>,
) -> Result<get_content_thumbnail::v3::Response> {
    let mxc = mxc_of(&body.server_name, &body.media_id);
    let dim = Dim::from_ruma(body.width, body.height, body.method.clone())?;
    let ours = services.server_state.server_is_ours(&body.server_name);

    let Media {
        content,
        content_type,
        content_disposition,
    } = if ours || body.allow_remote {
        fetch_thumbnail(&services, &mxc, body.timeout_ms, &dim).await?
    } else {
        let (meta, content) = services.media.get_thumbnail(&mxc, &dim).await?;

        media(meta, content, None)
    };

    Ok(get_content_thumbnail::v3::Response::new(
        content,
        content_type,
        content_disposition,
    ))
}

/// Loads a file, fetching it over federation only when `allow_remote` permits.
async fn fetch_legacy(
    services: &phantom_service::Services,
    mxc: &MxcUri,
    allow_remote: bool,
    timeout: std::time::Duration,
    filename: Option<&str>,
) -> Result<Media> {
    let (server_name, _) = mxc.parts()?;

    if services.server_state.server_is_ours(server_name) || allow_remote {
        return fetch_file(services, mxc, timeout, filename).await;
    }

    let (meta, content) = services
        .media
        .get(mxc, phantom_service::media::Dimensions::ORIGINAL)
        .await?;

    Ok(media(meta, content, filename))
}
