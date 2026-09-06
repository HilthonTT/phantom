use std::{
    net::IpAddr,
    time::{Duration, SystemTime},
};

use phantom_core::{Err, Result, debug, debug_warn, err, implement, time::timepoint_from_now};
#[cfg(feature = "url_preview")]
use reqwest::header::CONTENT_DISPOSITION;
use reqwest::{
    Url,
    header::{CONTENT_TYPE, COOKIE, HeaderValue, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use smallstr::SmallString;

use crate::media::Service;

/// A media type as declared by a page, inline for every common spelling.
type MediaType = SmallString<[u8; 32]>;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UrlPreviewData {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:title")]
    pub title: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:description"
    )]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:image")]
    pub image: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:image:size"
    )]
    pub image_size: Option<usize>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:image:width"
    )]
    pub image_width: Option<u32>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:image:height"
    )]
    pub image_height: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:video")]
    pub video: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:type"
    )]
    pub video_type: Option<MediaType>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:video:size"
    )]
    pub video_size: Option<usize>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:width"
    )]
    pub video_width: Option<u32>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "og:video:height"
    )]
    pub video_height: Option<u32>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:audio")]
    pub audio: Option<String>,

    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "matrix:audio:size"
    )]
    pub audio_size: Option<usize>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:type")]
    pub og_type: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "og:url")]
    pub og_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct CachedPreview {
    pub(super) preview: UrlPreviewData,
    pub(super) expires: SystemTime,
}

impl CachedPreview {
    fn new(ttl: Duration, preview: UrlPreviewData) -> Result<Self> {
        timepoint_from_now(ttl).map(|expires| Self { preview, expires })
    }

    #[inline]
    #[must_use]
    pub(super) fn is_expired(&self) -> bool {
        self.expires <= SystemTime::now()
    }

    #[inline]
    #[must_use]
    pub(super) fn is_valid(&self) -> bool {
        !self.is_expired()
    }
}

/// Which configured agent a preview request speaks as.
///
/// Origins commonly gate a page and the media it references differently, so
/// the two are configured separately.
#[derive(Clone, Copy)]
pub(super) enum Agent {
    Page,
    Media,
}

/// Hosts whose pages carry their `<head>` metadata only for an allowlisted
/// crawler, and which answer oEmbed for any agent.
const YOUTUBE_HOSTS: [&str; 5] = [
    "youtu.be",
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
];

/// Consent state that suppresses the interstitial Google serves in place of
/// the page in some regions.
///
/// `SOCS` is the cookie Google reads today and `CONSENT` the one older
/// endpoints still honor.
const YOUTUBE_CONSENT_COOKIE: &str = "SOCS=CAI; CONSENT=PENDING+999";

/// Endpoint answering oEmbed for every host in `YOUTUBE_HOSTS`, including
/// the short and subdomain forms.
#[cfg(feature = "url_preview")]
const YOUTUBE_OEMBED: &str = "https://www.youtube.com/oembed";

/// An oEmbed document runs to a few hundred bytes; the cap bounds only a
/// hostile origin.
#[cfg(feature = "url_preview")]
const OEMBED_MAX_SIZE: usize = 64 * 1024;

/// The oEmbed fields a preview can carry.
///
/// Every other field of the document is ignored, and each of these is
/// optional in the specification.
#[cfg(feature = "url_preview")]
#[derive(Deserialize)]
struct Oembed {
    #[serde(rename = "type")]
    kind: Option<String>,
    title: Option<String>,
    author_name: Option<String>,
    thumbnail_url: Option<String>,
}

#[implement(Service)]
pub async fn get_url_preview(&self, url: &Url) -> Result<UrlPreviewData> {
    if let Ok(cached) = self.db.get_url_preview(url.as_str()).await {
        if cached.is_valid() {
            return Ok(cached.preview);
        }
    }

    // Ensure that only one request is made per URL.
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

    // An upstream error response must not be turned into a cached preview.
    // Origins commonly gate pages and media differently by agent, so when a
    // distinct media agent is configured, a page-agent rejection is not
    // final: the URL may be a direct media link acceptable to the media
    // client.
    let status = response.status();

    let (response, via_media_client) = if status.is_success() {
        (response, false)
    } else if self.services.config.url_preview_media_user_agent.is_some() {
        (self.media_response(url).await?, true)
    } else {
        return Err!(Request(NotFound(debug_warn!("URL preview request failed"))));
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
            // Pages are only crawled with the page client; its rejection
            // stands even when the media client was served a page.
            if via_media_client {
                return Err!(Request(NotFound(debug_warn!("URL preview request failed"))));
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

    let ttl = Duration::from_secs(self.services.config.url_preview_cache_ttl);
    let cached = CachedPreview::new(ttl, data)?;

    self.db.set_url_preview(url.as_str(), &cached)?;

    Ok(cached.preview)
}

#[implement(Service)]
pub fn url_preview_allowed(&self, url: &Url) -> bool {
    if ["http", "https"]
        .iter()
        .all(|&scheme| !scheme.eq_ignore_ascii_case(url.scheme()))
    {
        debug!("Ignoring non-HTTP/HTTPS URL to preview: {}", url);
        return false;
    }

    let host = match url.host_str() {
        None => {
            debug!(
                "Ignoring URL preview for a URL that does not have a host (?): {}",
                url
            );
            return false;
        }
        Some(h) => h.to_owned(),
    };

    let allowlist_domain_contains = &self.services.config.url_preview_domain_contains_allowlist;
    let allowlist_domain_explicit = &self.services.config.url_preview_domain_explicit_allowlist;
    let denylist_domain_explicit = &self.services.config.url_preview_domain_explicit_denylist;
    let allowlist_url_contains = &self.services.config.url_preview_url_contains_allowlist;

    if allowlist_domain_contains.contains(&"*".to_owned())
        || allowlist_domain_explicit.contains(&"*".to_owned())
        || allowlist_url_contains.contains(&"*".to_owned())
    {
        debug!(
            "Config key contains * which is allowing all URL previews. Allowing URL {}",
            url
        );
        return true;
    }

    if !host.is_empty() {
        if denylist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is not allowed by url_preview_domain_explicit_denylist (check 1/4)",
                &host
            );
            return false;
        }

        if allowlist_domain_explicit.contains(&host) {
            debug!(
                "Host {} is allowed by url_preview_domain_explicit_allowlist (check 2/4)",
                &host
            );
            return true;
        }

        if allowlist_domain_contains
            .iter()
            .any(|domain_s| domain_s.contains(&host))
        {
            debug!(
                "Host {} is allowed by url_preview_domain_contains_allowlist (check 3/4)",
                &host
            );
            return true;
        }

        if allowlist_url_contains
            .iter()
            .any(|url_s| url.to_string().contains(url_s))
        {
            debug!(
                "URL {} is allowed by url_preview_url_contains_allowlist (check 4/4)",
                &host
            );
            return true;
        }

        if self.services.config.url_preview_check_root_domain {
            debug!("Checking root domain");

            match host.split_once('.') {
                None => return false,

                Some((_, root_domain)) => {
                    if denylist_domain_explicit.contains(root_domain) {
                        debug!(
                            "Root domain {} is not allowed by \
                             url_preview_domain_explicit_denylist (check 1/3)",
                            root_domain
                        );
                        return false;
                    }

                    if allowlist_domain_explicit.contains(root_domain) {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_explicit_allowlist \
                             (check 2/3)",
                            root_domain
                        );
                        return true;
                    }

                    if allowlist_domain_contains
                        .iter()
                        .any(|domain_s| domain_s.contains(root_domain))
                    {
                        debug!(
                            "Root domain {} is allowed by url_preview_domain_contains_allowlist \
                             (check 3/3)",
                            root_domain
                        );
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Whether an OpenGraph media object's declared type belongs to `class`.
///
/// A missing `og:*:type` is accepted, since most origins omit it. A type
/// outside the class means the URL addresses a player page rather than a
/// file, which the relay cannot serve as media.
#[cfg(feature = "url_preview")]
fn declares_media_type(obj: &OpengraphObject, class: &str) -> bool {
    obj.properties
        .get("type")
        .is_none_or(|kind| kind.starts_with(class))
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
async fn download_html(&self, _url: &Url, _response: reqwest::Response) -> Result<UrlPreviewData> {
    Err!(FeatureDisabled("url_preview"))
}

/// Read a page body up to `limit`, reporting whether the cap cut it short.
///
/// An advertised length seeds the buffer, and growth past that stays
/// geometric but never exceeds the cap, so a truncated page costs the cap
/// rather than the next power of two above it.
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

/// Reserve `want` more bytes, growing geometrically but never past `limit`.
///
/// `want` is expected to be clamped to the remaining budget by the caller; a
/// larger value is honored rather than dropped, since refusing to reserve it
/// would only move the allocation into the following `extend_from_slice`.
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

/// Mint an `mxc://` URI for a page's declared media, or nothing when it is not
/// relayable.
///
/// The URL is recorded rather than fetched, so a page naming a large video
/// costs the preview request no bandwidth; it is fetched and checked only once
/// a client asks for the resulting URI. Screening IP literals here as well
/// keeps a preview from handing out a URI that the same check at relay time is
/// guaranteed to refuse.
#[cfg(feature = "url_preview")]
#[implement(Service)]
fn lazy_media(&self, page: &Url, obj: &OpengraphObject, class: &str) -> Option<String> {
    declares_media_type(obj, class)
        .then(|| page.join(&obj.url).ok())
        .flatten()
        .filter(|url| ["http", "https"].contains(&url.scheme()))
        .filter(|url| self.check_url_host(url).is_ok())
        .map(|url| self.register_lazy_media(url.as_str()))
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
async fn download_html(&self, url: &Url, response: reqwest::Response) -> Result<UrlPreviewData> {
    use webpage::HTML;

    let limit = self.services.config.url_preview_max_spider_size;
    let (bytes, truncated) = spider_body(response, limit).await?;

    // the parser needs an owned string, so the read buffer becomes one rather
    // than being copied into a second buffer of the same size
    let body = String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    let Ok(html) = HTML::from_string(body, Some(url.as_str().to_owned())) else {
        return Err!(Request(Unknown("Failed to parse HTML")));
    };

    // twitter:* card tags mirror og:; some pages emit only the twitter set,
    // or (fixvx) an empty og: value beside the real twitter: one
    let twitter = |key| {
        html.meta
            .get(key)
            .map(String::as_str)
            .filter(|content| !content.is_empty())
    };

    // `webpage` does not resolve relative URLs in `og:` meta tags; resolve
    // against the page URL, then keep only the http(s) ones we can fetch
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
        // the declared type is reported even when the URL cannot be relayed
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

    // a page whose head metadata sits past the cap parses clean and yields
    // nothing, which is indistinguishable from a page carrying no tags
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

/// Mint a local mxc:// URI that resolves to `url` on first download (see
/// `Service::fetch_lazy_media`), keeping preview generation independent of the
/// underlying file size while routing clients through this server.
#[cfg(feature = "url_preview")]
#[implement(Service)]
fn register_lazy_media(&self, url: &str) -> String {
    let mxc = self.mint_lazy_media();

    self.db.insert_lazy_media(&mxc, url);

    mxc
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
fn queue_lazy_media(&self, txn: &mut Txn, url: &str) -> String {
    let mxc = self.mint_lazy_media();

    self.db.queue_lazy_media(txn, &mxc, url);

    mxc
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
fn mint_lazy_media(&self) -> String {
    Mxc {
        server_name: self.services.globals.server_name(),
        media_id: &random_string(MXC_LENGTH),
    }
    .to_string()
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub async fn download_video(&self, response: reqwest::Response) -> Result<UrlPreviewData> {
    let video_size =
        checked_media_size(&response, self.services.config.url_preview_max_media_size)?;

    Ok(UrlPreviewData {
        video: Some(self.register_lazy_media(response.url().as_str())),
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
    let audio_size =
        checked_media_size(&response, self.services.config.url_preview_max_media_size)?;

    Ok(UrlPreviewData {
        audio: Some(self.register_lazy_media(response.url().as_str())),
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

/// Parse a direct-file preview's advertised size, refusing one over the cap so
/// we never register an mxc the relay is guaranteed to reject at fetch time.
#[cfg(feature = "url_preview")]
fn checked_media_size(response: &reqwest::Response, limit: usize) -> Result<Option<usize>> {
    let size = response
        .content_length()
        .and_then(|len| usize::try_from(len).ok());

    if size.is_some_and(|size| size > limit) {
        return Err!(Request(TooLarge(
            "Media exceeds url_preview_max_media_size"
        )));
    }

    Ok(size)
}

#[implement(Service)]
pub(super) fn check_url_host(&self, url: &Url) -> Result {
    if self.services.client.proxy.resolver_alias(url) {
        return Err!(Request(Forbidden(
            "Requesting a locally resolved proxy endpoint is forbidden"
        )));
    }

    let host = url
        .host()
        .ok_or_else(|| err!(Request(Unknown("URL has no host"))))?;

    let ip = match host {
        Host::Domain(_) => return Ok(()),
        Host::Ipv4(v4) => IpAddr::V4(v4),
        Host::Ipv6(v6) => IpAddr::V6(v6),
    };

    if !self.services.client.valid_cidr_range_ip(ip) {
        return Err!(Request(Forbidden(
            "Requesting from this address is forbidden"
        )));
    }

    Ok(())
}

/// Verify a possibly-refetched preview response still carries the content type
/// class the page response was dispatched on, so a media-client refetch that
/// substitutes a different type is not mis-registered.
fn require_media_type(response: &reqwest::Response, class: &str) -> Result {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with(class))
        .then_some(())
        .ok_or_else(|| err!(Request(Unknown("Unsupported Content-Type"))))
}
