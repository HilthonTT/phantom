use phantom_core::implement;
#[cfg(feature = "url_preview")]
use phantom_core::{Err, Result, debug, err};
use reqwest::Url;
#[cfg(feature = "url_preview")]
use serde::Deserialize;

use super::UrlPreviewData;
#[cfg(feature = "url_preview")]
use super::{Agent, request::is_youtube};
#[cfg(feature = "url_preview")]
use crate::client::read_response_capped;
use crate::media::Service;

#[cfg(feature = "url_preview")]
const YOUTUBE_OEMBED: &str = "https://www.youtube.com/oembed";

#[cfg(feature = "url_preview")]
const OEMBED_MAX_SIZE: usize = 64 * 1024;

#[cfg(feature = "url_preview")]
#[derive(Deserialize)]
struct Oembed {
    #[serde(rename = "type")]
    kind: Option<String>,
    title: Option<String>,
    author_name: Option<String>,
    thumbnail_url: Option<String>,
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub(super) async fn oembed_recover(&self, url: &Url, data: UrlPreviewData) -> UrlPreviewData {
    if data.title.is_some() || data.image.is_some() {
        return data;
    }

    let Some(endpoint) = oembed_endpoint(url) else {
        return data;
    };

    self.oembed_preview(&endpoint, url)
        .await
        .inspect_err(|e| debug!(%url, %e, "oEmbed recovery failed"))
        .unwrap_or(data)
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub(super) async fn oembed_recover(&self, _url: &Url, data: UrlPreviewData) -> UrlPreviewData {
    data
}

#[cfg(feature = "url_preview")]
fn oembed_endpoint(url: &Url) -> Option<Url> {
    is_youtube(url)
        .then(|| {
            Url::parse_with_params(YOUTUBE_OEMBED, [("url", url.as_str()), ("format", "json")])
        })
        .and_then(Result::ok)
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
async fn oembed_preview(&self, endpoint: &Url, page: &Url) -> Result<UrlPreviewData> {
    if !self.url_preview_allowed(endpoint) {
        return Err!(Request(Forbidden(debug_warn!(
            "oEmbed endpoint {endpoint} is not allowed for previewing"
        ))));
    }

    self.check_url_host(endpoint)?;

    let response = self.preview_get(endpoint, Agent::Page).send().await?;

    self.check_remote_addr(&response)?;

    let status = response.status();

    if !status.is_success() {
        return Err!(Request(NotFound(debug_warn!(
            "oEmbed request to {endpoint} failed: {status}"
        ))));
    }

    let body = read_response_capped(response, OEMBED_MAX_SIZE).await?;

    let oembed: Oembed = serde_json::from_slice(&body)
        .map_err(|e| err!(Request(Unknown("Invalid oEmbed document: {e}"))))?;

    let image = self.oembed_image(oembed.thumbnail_url.as_deref()).await;

    Ok(UrlPreviewData {
        title: oembed.title,
        description: oembed.author_name,
        video_type: video_type(oembed.kind.as_deref()).map(Into::into),
        og_type: og_type(oembed.kind.as_deref()),
        og_url: Some(page.as_str().to_owned()),
        ..image
    })
}

#[cfg(feature = "url_preview")]
fn video_type(kind: Option<&str>) -> Option<&'static str> {
    kind.eq(&Some("video")).then_some("text/html")
}

#[cfg(feature = "url_preview")]
fn og_type(kind: Option<&str>) -> Option<String> {
    kind.map(|kind| match kind {
        "video" => "video.other",
        _ => "website",
    })
    .map(ToOwned::to_owned)
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
async fn oembed_image(&self, thumbnail_url: Option<&str>) -> UrlPreviewData {
    let Some(thumbnail) = thumbnail_url
        .and_then(|thumbnail| Url::parse(thumbnail).ok())
        .filter(|thumbnail| ["http", "https"].contains(&thumbnail.scheme()))
    else {
        return UrlPreviewData::default();
    };

    self.preview_image(&thumbnail).await.unwrap_or_default()
}

#[cfg(all(test, feature = "url_preview"))]
mod tests {
    use reqwest::Url;

    use super::{oembed_endpoint, video_type};

    fn url(url: &str) -> Url {
        Url::parse(url).expect("parses")
    }

    #[test]
    fn oembed_endpoint_carries_the_page_url() {
        let page = url("https://www.youtube.com/watch?v=a&b=c");
        let endpoint = oembed_endpoint(&page).expect("youtube has an endpoint");

        assert_eq!(endpoint.path(), "/oembed");

        let params: Vec<_> = endpoint.query_pairs().collect();

        assert_eq!(
            params,
            [
                ("url".into(), page.as_str().into()),
                ("format".into(), "json".into())
            ]
        );

        assert!(oembed_endpoint(&url("https://example.org/")).is_none());
    }

    #[test]
    fn oembed_video_declares_a_player() {
        assert_eq!(video_type(Some("video")), Some("text/html"));

        for kind in [Some("photo"), Some("rich"), Some("link"), None] {
            assert!(video_type(kind).is_none(), "{kind:?}");
        }
    }
}
