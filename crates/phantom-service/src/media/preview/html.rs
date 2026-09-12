use phantom_core::{Err, Result, implement};
#[cfg(feature = "url_preview")]
use phantom_core::{debug_warn, err};
use reqwest::Url;

use super::UrlPreviewData;
use crate::media::Service;

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub(super) async fn download_html(
    &self,
    url: &Url,
    response: reqwest::Response,
) -> Result<UrlPreviewData> {
    use webpage::HTML;

    let limit = self.services.config.media.url_preview_max_spider_size;
    let (bytes, truncated) = spider_body(response, limit).await?;

    let body = String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    let Ok(html) = HTML::from_string(body, Some(url.as_str().to_owned())) else {
        return Err!(Request(Unknown("Failed to parse HTML")));
    };

    let twitter = |key| {
        html.meta
            .get(key)
            .map(String::as_str)
            .filter(|content| !content.is_empty())
    };

    let image_url = html
        .opengraph
        .images
        .first()
        .map(|obj| obj.url.as_str())
        .filter(|image| !image.is_empty())
        .or_else(|| twitter("twitter:image"))
        .or_else(|| twitter("twitter:image:src"))
        .map(|image| url.join(image))
        .transpose()
        .map_err(|e| err!(Request(Unknown("Invalid preview image URL: {e}"))))?
        .filter(|image_url| ["http", "https"].contains(&image_url.scheme()));

    let mut data = match image_url {
        None => UrlPreviewData::default(),
        Some(image_url) => self.preview_image(&image_url).await?,
    };

    if let Some(obj) = html.opengraph.videos.first()
        && !obj.url.is_empty()
    {
        data.video_type = obj
            .properties
            .get("type")
            .map(String::as_str)
            .map(Into::into);

        data.video_width = obj.properties.get("width").and_then(|w| w.parse().ok());

        data.video_height = obj.properties.get("height").and_then(|h| h.parse().ok());

        data.video = self.lazy_media(url, obj, "video/");
    }

    if let Some(obj) = html.opengraph.audios.first()
        && !obj.url.is_empty()
    {
        data.audio = self.lazy_media(url, obj, "audio/");
    }

    let props = html.opengraph.properties;

    data.title = props
        .get("title")
        .cloned()
        .filter(|title| !title.is_empty())
        .or_else(|| twitter("twitter:title").map(ToOwned::to_owned))
        .or(html.title);

    data.description = props
        .get("description")
        .cloned()
        .filter(|description| !description.is_empty())
        .or_else(|| twitter("twitter:description").map(ToOwned::to_owned))
        .or(html.description);

    data.og_type = Some(html.opengraph.og_type);
    data.og_url = props.get("url").cloned();

    if truncated && data.title.is_none() && data.description.is_none() && data.image.is_none() {
        debug_warn!(
            %url,
            %limit,
            "Preview page was truncated before any metadata was found; a larger \
             url_preview_max_spider_size or a different url_preview_user_agent may be needed"
        );
    }

    Ok(data)
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub(super) async fn download_html(
    &self,
    _url: &Url,
    _response: reqwest::Response,
) -> Result<UrlPreviewData> {
    Err!(FeatureDisabled("url_preview"))
}

#[cfg(feature = "url_preview")]
async fn spider_body(mut response: reqwest::Response, limit: usize) -> Result<(Vec<u8>, bool)> {
    let hint = response
        .content_length()
        .and_then(|len| usize::try_from(len).ok())
        .map_or(0, |len| len.min(limit));

    let mut bytes: Vec<u8> = Vec::with_capacity(hint);

    while let Some(chunk) = response.chunk().await? {
        let want = chunk.len().min(limit.saturating_sub(bytes.len()));

        reserve_capped(&mut bytes, want, limit);
        bytes.extend_from_slice(&chunk[..want]);

        if want < chunk.len() {
            return Ok((bytes, true));
        }
    }

    Ok((bytes, false))
}

#[cfg(feature = "url_preview")]
fn reserve_capped(bytes: &mut Vec<u8>, want: usize, limit: usize) {
    let need = bytes.len().saturating_add(want);

    if need <= bytes.capacity() {
        return;
    }

    let target = bytes
        .capacity()
        .saturating_mul(2)
        .clamp(need, limit.max(need));

    bytes.reserve_exact(target.saturating_sub(bytes.len()));
}

#[cfg(all(test, feature = "url_preview"))]
mod tests {
    use super::reserve_capped;

    #[test]
    fn reserve_capped_never_exceeds_the_cap() {
        const LIMIT: usize = 768 * 1024;

        let mut bytes: Vec<u8> = Vec::new();
        let chunk = vec![0_u8; 16 * 1024];
        let mut reallocs = 0;

        while bytes.len() < LIMIT {
            let want = chunk.len().min(LIMIT.saturating_sub(bytes.len()));
            let before = bytes.capacity();

            reserve_capped(&mut bytes, want, LIMIT);
            bytes.extend_from_slice(&chunk[..want]);

            if bytes.capacity() != before {
                reallocs += 1;
            }

            assert!(
                bytes.capacity() <= LIMIT,
                "capacity {} past cap",
                bytes.capacity()
            );
        }

        assert_eq!(bytes.len(), LIMIT);

        assert!(reallocs < 12, "{reallocs} reallocations");
    }

    #[test]
    fn reserve_capped_honors_an_unclamped_request() {
        let mut bytes: Vec<u8> = Vec::new();

        reserve_capped(&mut bytes, 64, 16);

        assert!(bytes.capacity() >= 64);
    }
}
