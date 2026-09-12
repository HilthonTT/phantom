#[cfg(not(feature = "url_preview"))]
use phantom_core::Err;
#[cfg(feature = "url_preview")]
use phantom_core::debug;
use phantom_core::{Result, implement};
#[cfg(feature = "url_preview")]
use reqwest::{
    Url,
    header::{CONTENT_DISPOSITION, CONTENT_TYPE},
};

use super::UrlPreviewData;
#[cfg(feature = "url_preview")]
use super::{Agent, request::checked_media_size};
#[cfg(feature = "url_preview")]
use crate::client::read_response_capped;
use crate::media::Service;

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub(super) async fn preview_image(&self, image_url: &Url) -> Result<UrlPreviewData> {
    self.check_url_host(image_url)?;

    let response = self.preview_get(image_url, Agent::Media).send().await?;

    self.check_remote_addr(&response)?;

    if !response.status().is_success() {
        debug!(
            %image_url,
            status = ?response.status(),
            "Skipping preview image with unsuccessful response"
        );

        return Ok(UrlPreviewData::default());
    }

    self.download_image(response).await
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub async fn download_image(&self, response: reqwest::Response) -> Result<UrlPreviewData> {
    use image::ImageReader;

    let url = response.url().clone();

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    let content_disposition = response
        .headers()
        .get(CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    let limit = self.services.config.media.url_preview_max_media_size;
    let image = read_response_capped(response, limit).await?;

    let cursor = std::io::Cursor::new(&image);

    let (width, height) = match ImageReader::new(cursor).with_guessed_format() {
        Err(_) => (None, None),
        Ok(reader) => match reader.into_dimensions() {
            Err(_) => (None, None),
            Ok((width, height)) => (Some(width), Some(height)),
        },
    };

    let mut txn = self.db.txn();

    let mxc = self.queue_lazy_media(&mut txn, url.as_str());

    self.db.set_lazy_content(
        &mut txn,
        &mxc,
        content_type.as_deref(),
        content_disposition.as_deref(),
        &image,
    )?;

    txn.execute()?;

    Ok(UrlPreviewData {
        image: Some(mxc),
        image_size: Some(image.len()),
        image_width: width,
        image_height: height,
        ..Default::default()
    })
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_image(&self, _response: reqwest::Response) -> Result<UrlPreviewData> {
    Err!(FeatureDisabled("url_preview"))
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_video(&self, response: reqwest::Response) -> Result<UrlPreviewData> {
    let video_size = checked_media_size(
        &response,
        self.services.config.media.url_preview_max_media_size,
    )?;

    Ok(UrlPreviewData {
        video: Some(self.register_lazy_media(response.url().as_str())?),
        video_size,
        ..Default::default()
    })
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_video(&self, _response: reqwest::Response) -> Result<UrlPreviewData> {
    Err!(FeatureDisabled("url_preview"))
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_audio(&self, response: reqwest::Response) -> Result<UrlPreviewData> {
    let audio_size = checked_media_size(
        &response,
        self.services.config.media.url_preview_max_media_size,
    )?;

    Ok(UrlPreviewData {
        audio: Some(self.register_lazy_media(response.url().as_str())?),
        audio_size,
        ..Default::default()
    })
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_audio(&self, _response: reqwest::Response) -> Result<UrlPreviewData> {
    Err!(FeatureDisabled("url_preview"))
}
