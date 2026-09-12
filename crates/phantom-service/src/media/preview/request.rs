use phantom_core::{Err, Result, err, implement};
use reqwest::{
    Url,
    header::{CONTENT_TYPE, COOKIE, HeaderValue, USER_AGENT},
};

use crate::media::Service;
#[cfg(feature = "url_preview")]
use crate::{client::read_response_capped, media::Media};

#[derive(Clone, Copy)]
pub(super) enum Agent {
    Page,

    #[cfg_attr(
        not(feature = "url_preview"),
        expect(
            dead_code,
            reason = "nothing fetches preview media without the feature"
        )
    )]
    Media,
}

const YOUTUBE_HOSTS: [&str; 5] = [
    "youtu.be",
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
];

const YOUTUBE_CONSENT_COOKIE: &str = "SOCS=CAI; CONSENT=PENDING+999";

#[implement(Service)]
pub(super) fn preview_get(&self, url: &Url, agent: Agent) -> reqwest::RequestBuilder {
    let request = self.services.client.url_preview.get(url.as_str());

    self.preview_headers(request, url, agent)
}

#[implement(Service)]
fn preview_headers(
    &self,
    request: reqwest::RequestBuilder,
    url: &Url,
    agent: Agent,
) -> reqwest::RequestBuilder {
    let config = &self.services.config;

    let user_agent = match agent {
        Agent::Page => config.media.url_preview_user_agent.as_deref(),
        Agent::Media => config
            .media
            .url_preview_media_user_agent
            .as_deref()
            .or(config.media.url_preview_user_agent.as_deref()),
    };

    let request = match user_agent {
        Some(user_agent) => request.header(USER_AGENT, user_agent),
        None => request,
    };

    match is_youtube(url) {
        true => request.header(COOKIE, HeaderValue::from_static(YOUTUBE_CONSENT_COOKIE)),
        false => request,
    }
}

#[must_use]
pub(super) fn is_youtube(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| YOUTUBE_HOSTS.contains(&host))
}

pub(super) fn require_media_type(response: &reqwest::Response, class: &str) -> Result {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with(class))
        .then_some(())
        .ok_or_else(|| err!(Request(Unknown("Unsupported Content-Type"))))
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub(super) async fn media_response(&self, url: &Url) -> Result<reqwest::Response> {
    let response = self.preview_get(url, Agent::Media).send().await?;

    self.check_remote_addr(&response)?;

    let status = response.status();

    if !status.is_success() {
        return Err!(Request(NotFound(debug_warn!(
            "URL preview media request for {url} failed: {status}"
        ))));
    }

    Ok(response)
}

#[cfg(not(feature = "url_preview"))]
#[implement(Service)]
#[expect(clippy::unused_async)]
pub(super) async fn media_response(&self, _url: &Url) -> Result<reqwest::Response> {
    Err!(FeatureDisabled("url_preview"))
}

#[implement(Service)]
pub(super) async fn media_refetch(
    &self,
    url: &Url,
    response: reqwest::Response,
    via_media_client: bool,
) -> Result<reqwest::Response> {
    if via_media_client
        || self
            .services
            .config
            .media
            .url_preview_media_user_agent
            .is_none()
    {
        return Ok(response);
    }

    self.media_response(url).await
}

#[cfg(feature = "url_preview")]
#[implement(Service)]
pub(in crate::media) async fn fetch_preview_media(&self, url: &Url) -> Result<Media> {
    use ruma::http_headers::ContentDisposition;

    self.check_url_host(url)?;

    let response = self.preview_get(url, Agent::Media).send().await?;

    self.check_remote_addr(&response)?;

    let status = response.status();

    if !status.is_success() {
        return Err!(Request(NotFound(debug_warn!(
            "Preview media request for {url} failed: {status}"
        ))));
    }

    let limit = self.services.config.media.url_preview_max_media_size;

    checked_media_size(&response, limit)?;

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    let content_disposition = ContentDisposition::new(
        phantom_core::content_disposition::content_disposition_type(content_type.as_deref()),
    );

    let content = read_response_capped(response, limit).await?;

    Ok(Media {
        content: content.to_vec(),
        content_type,
        content_disposition: Some(content_disposition),
    })
}

#[cfg(feature = "url_preview")]
pub(super) fn checked_media_size(
    response: &reqwest::Response,
    limit: usize,
) -> Result<Option<usize>> {
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

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::is_youtube;

    fn url(url: &str) -> Url {
        Url::parse(url).expect("parses")
    }

    #[test]
    fn youtube_hosts_matched() {
        let youtube = [
            "https://www.youtube.com/watch?v=abc",
            "https://youtu.be/abc",
            "https://music.youtube.com/watch?v=abc",
            "https://m.youtube.com/watch?v=abc",
            "https://youtube.com/watch?v=abc",
            "https://WWW.YOUTUBE.COM/watch?v=abc",
        ];

        for page in youtube {
            assert!(is_youtube(&url(page)), "{page}");
        }

        let other = [
            "https://youtube.com.evil.example/watch?v=abc",
            "https://notyoutube.com/watch?v=abc",
            "https://i.ytimg.com/vi/abc/hqdefault.jpg",
            "https://example.org/",
        ];

        for page in other {
            assert!(!is_youtube(&url(page)), "{page}");
        }
    }
}
