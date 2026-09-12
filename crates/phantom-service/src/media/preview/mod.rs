mod download;
mod html;
#[cfg(feature = "url_preview")]
mod lazy;
mod model;
mod oembed;
mod policy;
mod request;

use std::time::Duration;

use phantom_core::{Err, Result, debug, err, implement};
use reqwest::{Url, header::CONTENT_TYPE};

pub(in crate::media) use self::model::CachedPreview;
pub use self::model::UrlPreviewData;
use self::request::{Agent, require_media_type};
use crate::media::Service;

#[implement(Service)]
pub async fn get_url_preview(&self, url: &Url) -> Result<UrlPreviewData> {
    if let Ok(cached) = self.db.get_url_preview(url.as_str()).await
        && cached.is_valid()
    {
        return Ok(cached.preview);
    }

    let _request_lock = self.url_preview_mutex.lock(url.as_str()).await;

    match self.db.get_url_preview(url.as_str()).await {
        Ok(cached) if cached.is_valid() => Ok(cached.preview),
        _ => self.request_url_preview(url).await,
    }
}

#[implement(Service)]
pub async fn request_url_preview(&self, url: &Url) -> Result<UrlPreviewData> {
    self.check_url_host(url)?;

    let response = self.preview_get(url, Agent::Page).send().await?;

    debug!(
        ?url,
        "URL preview response headers: {:?}",
        response.headers()
    );

    self.check_remote_addr(&response)?;

    let status = response.status();

    let (response, via_media_client) = if status.is_success() {
        (response, false)
    } else if self
        .services
        .config
        .media
        .url_preview_media_user_agent
        .is_some()
    {
        (self.media_response(url).await?, true)
    } else {
        return Err!(Request(NotFound(debug_warn!(
            "URL preview request for {url} failed: {status}"
        ))));
    };

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .ok_or_else(|| err!(Request(Unknown("Missing Content-Type header"))))?
        .to_str()
        .map_err(|e| err!(Request(Unknown("Invalid Content-Type header: {e}"))))?
        .to_owned();

    let data = match content_type.as_str() {
        html if html.starts_with("text/html") => {
            if via_media_client {
                return Err!(Request(NotFound(debug_warn!(
                    "URL preview request for {url} failed: {status}"
                ))));
            }

            let data = self.download_html(url, response).await?;

            self.oembed_recover(url, data).await
        }

        img if img.starts_with("image/") => {
            let response = self.media_refetch(url, response, via_media_client).await?;

            require_media_type(&response, "image/")?;
            self.download_image(response).await?
        }

        video if video.starts_with("video/") => {
            let response = self.media_refetch(url, response, via_media_client).await?;

            require_media_type(&response, "video/")?;
            self.download_video(response).await?
        }

        audio if audio.starts_with("audio/") => {
            let response = self.media_refetch(url, response, via_media_client).await?;

            require_media_type(&response, "audio/")?;
            self.download_audio(response).await?
        }

        _ => return Err!(Request(Unknown("Unsupported Content-Type"))),
    };

    let ttl = Duration::from_secs(self.services.config.media.url_preview_cache_ttl);
    let cached = CachedPreview::new(ttl, data)?;

    self.db.set_url_preview(url.as_str(), &cached)?;

    Ok(cached.preview)
}
