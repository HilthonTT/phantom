use std::str::FromStr;

use axum::extract::State;
use phantom_core::{Result, content_disposition::make_content_disposition};
use phantom_service::media::{Dim, Dimensions, FileMeta};
use ruma::{
    OwnedMxcUri,
    api::federation::authenticated_media::{
        Content, ContentMetadata, FileOrLocation, get_content, get_content_thumbnail,
    },
    http_headers::ContentDisposition,
};

use crate::router::{ClientIp, Ruma};

#[tracing::instrument(name = "media_get", level = "debug", skip_all, fields(%client))]
pub(crate) async fn get_content_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content::v1::Request>,
) -> Result<get_content::v1::Response> {
    let mxc = local_mxc(&services, &body.media_id);
    let (meta, file) = services.media.get(&mxc, Dimensions::ORIGINAL).await?;

    Ok(get_content::v1::Response::new(
        ContentMetadata::new(),
        file_content(meta, file),
    ))
}

#[tracing::instrument(name = "media_thumbnail_get", level = "debug", skip_all, fields(%client))]
pub(crate) async fn get_content_thumbnail_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_content_thumbnail::v1::Request>,
) -> Result<get_content_thumbnail::v1::Response> {
    let dim = Dim::from_ruma(body.width, body.height, body.method.clone())?;
    let mxc = local_mxc(&services, &body.media_id);
    let (meta, file) = services.media.get_thumbnail(&mxc, &dim).await?;

    Ok(get_content_thumbnail::v1::Response::new(
        ContentMetadata::new(),
        file_content(meta, file),
    ))
}

fn local_mxc(services: &phantom_service::Services, media_id: &str) -> OwnedMxcUri {
    let server_name = services.server_state.server_name();

    OwnedMxcUri::from(format!("mxc://{server_name}/{media_id}"))
}

fn file_content(meta: FileMeta, file: Vec<u8>) -> FileOrLocation {
    let content_disposition = meta
        .content_disposition
        .as_deref()
        .and_then(|value| ContentDisposition::from_str(value).ok());

    let content_disposition = make_content_disposition(
        content_disposition.as_ref(),
        meta.content_type.as_deref(),
        None,
    );

    let content = Content::new(
        file,
        meta.content_type
            .unwrap_or_else(|| "application/octet-stream".to_owned()),
        content_disposition,
    );
    FileOrLocation::File(content)
}
