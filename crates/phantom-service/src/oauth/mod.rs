pub mod providers;
pub mod server;
pub mod sessions;
pub mod token_response;
pub mod user_info;

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as b64encode};
use futures::{Stream, StreamExt, TryStreamExt};
use phantom_core::{
    Err, Error, Result, err, hash::sha256, http::StatusCode, implement, result::LogErr,
    stream::ReadyExt,
};
use reqwest::{
    Method,
    header::{ACCEPT, CONTENT_TYPE},
};
use ruma::{
    UserId,
    api::error::{ErrorKind, LimitExceededErrorData},
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use url::Url;

use self::{providers::Providers, sessions::Sessions};
pub use self::{
    providers::{Provider, ProviderId},
    server::Server,
    sessions::{CODE_VERIFIER_LENGTH, SESSION_ID_LENGTH, Session, SessionId},
    token_response::TokenResponse,
    user_info::UserInfo,
};
use crate::{Dep, client, client::read_response_capped, config};

type Ratelimiter = Mutex<HashMap<IpAddr, (Instant, f64)>>;

pub struct Service {
    services: Services,

    pub providers: Arc<Providers>,
    pub sessions: Arc<Sessions>,

    pub server: Option<Arc<Server>>,

    ratelimiter: Ratelimiter,
    device_ratelimiter: Ratelimiter,
}

struct Services {
    client: Dep<client::Service>,
    config: Dep<config::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        let providers = Arc::new(Providers::build(&args));
        let sessions = Arc::new(Sessions::build(&args, providers.clone()));
        let server = Server::build(&args)?.map(Arc::new);

        Ok(Arc::new(Self {
            services: Services {
                client: args.depend::<client::Service>("client"),
                config: args.depend::<config::Service>("config"),
            },
            providers,
            sessions,
            server,
            ratelimiter: Mutex::new(HashMap::new()),
            device_ratelimiter: Mutex::new(HashMap::new()),
        }))
    }

    async fn clear_cache(&self) {
        self.providers.clear_cache().await;
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
#[inline]
pub fn get_server(&self) -> Result<&Server> {
    self.server
        .as_deref()
        .ok_or_else(|| err!(Request(Unrecognized("The OIDC server is not configured"))))
}

const RATELIMIT_MAP_CAP: usize = 1 << 16;

const DEVICE_RC_PER_SECOND: f64 = 1.0;
const DEVICE_RC_BURST: f64 = 60.0;

#[implement(Service)]
pub fn check_rate_limit(&self, client: IpAddr) -> Result {
    let config = &self.services.config;
    let rate = f64::from(config.oidc.oidc_rc_per_second);
    let burst = f64::from(config.oidc.oidc_rc_burst_count);

    if rate <= 0.0 || burst <= 0.0 {
        return Ok(());
    }

    check_bucket(&self.ratelimiter, client, rate, burst)
}

#[implement(Service)]
pub fn check_device_rate_limit(&self, client: IpAddr) -> Result {
    check_bucket(
        &self.device_ratelimiter,
        client,
        DEVICE_RC_PER_SECOND,
        DEVICE_RC_BURST,
    )
}

fn check_bucket(table: &Ratelimiter, client: IpAddr, rate: f64, burst: f64) -> Result {
    let now = Instant::now();
    let mut buckets = table.lock()?;

    if buckets.len() >= RATELIMIT_MAP_CAP {
        buckets.retain(|_, (last, tokens)| {
            now.duration_since(*last)
                .as_secs_f64()
                .mul_add(rate, *tokens)
                < burst
        });
    }

    let (last_time, tokens) = buckets.entry(client).or_insert_with(|| (now, burst));

    let new_tokens = now
        .duration_since(*last_time)
        .as_secs_f64()
        .mul_add(rate, *tokens)
        .min(burst);

    if new_tokens < 1.0 {
        return Err(Error::Request(
            ErrorKind::LimitExceeded(LimitExceededErrorData::new()),
            "Too many OIDC requests.".into(),
            StatusCode::TOO_MANY_REQUESTS,
        ));
    }

    *last_time = now;
    *tokens = new_tokens - 1.0;

    Ok(())
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn delete_user_sessions(&self, user_id: &UserId) {
    let sess_ids: Vec<_> = self
        .user_sessions(user_id)
        .ready_filter_map(Result::ok)
        .ready_filter_map(|(_, session)| session.sess_id)
        .collect()
        .await;

    for sess_id in &sess_ids {
        self.sessions.delete(sess_id).await.log_err().ok();
    }
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn revoke_user_tokens(&self, user_id: &UserId) {
    let sessions: Vec<_> = self
        .user_sessions(user_id)
        .ready_filter_map(Result::ok)
        .collect()
        .await;

    for (provider, session) in &sessions {
        self.revoke_token((provider, session)).await.log_err().ok();
    }
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub fn user_sessions(
    &self,
    user_id: &UserId,
) -> impl Stream<Item = Result<(Provider, Session)>> + Send {
    self.sessions
        .get_by_user(user_id)
        .and_then(async |session| Ok((self.sessions.provider(&session).await?, session)))
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, ret)]
pub async fn request_userinfo(
    &self,
    (provider, session): (&Provider, &Session),
) -> Result<UserInfo> {
    let url = provider
        .userinfo_url
        .clone()
        .ok_or_else(|| err!(Config("userinfo_url", "Missing userinfo URL in config")))?;

    self.request(
        (Some(provider), Some(session)),
        Method::GET,
        url,
        Option::<()>::None,
    )
    .await
    .and_then(|value| serde_json::from_value(value).map_err(Into::into))
    .log_err()
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, ret)]
pub async fn request_tokeninfo(
    &self,
    (provider, session): (&Provider, &Session),
) -> Result<UserInfo> {
    let url = provider.introspection_url.clone().ok_or_else(|| {
        err!(Config(
            "introspection_url",
            "Missing introspection URL in config"
        ))
    })?;

    self.request(
        (Some(provider), Some(session)),
        Method::GET,
        url,
        Option::<()>::None,
    )
    .await
    .and_then(|value| serde_json::from_value(value).map_err(Into::into))
    .log_err()
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, ret)]
pub async fn revoke_token(&self, (provider, session): (&Provider, &Session)) -> Result {
    #[derive(Debug, Serialize)]
    struct RevokeQuery<'a> {
        client_id: &'a str,
        client_secret: &'a str,
    }

    let client_secret = self.providers.get_client_secret(provider).await?;

    let query = RevokeQuery {
        client_id: &provider.client_id,
        client_secret: &client_secret,
    };

    let url = provider
        .revocation_url
        .clone()
        .ok_or_else(|| err!(Config("revocation_url", "Missing revocation URL in config")))?;

    self.request(
        (Some(provider), Some(session)),
        Method::POST,
        url,
        Some(query),
    )
    .await
    .log_err()
    .map(|_| ())
}

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, ret)]
pub async fn request_token(
    &self,
    (provider, session): (&Provider, &Session),
    code: &str,
) -> Result<TokenResponse> {
    #[derive(Debug, Serialize)]
    struct TokenQuery<'a> {
        client_id: &'a str,
        client_secret: &'a str,
        grant_type: &'a str,
        code: &'a str,
        code_verifier: Option<&'a str>,
        redirect_uri: Option<&'a str>,
    }

    let client_secret = self.providers.get_client_secret(provider).await?;

    let query = TokenQuery {
        client_id: &provider.client_id,
        client_secret: &client_secret,
        grant_type: "authorization_code",
        code,
        code_verifier: session.code_verifier.as_deref(),
        redirect_uri: provider.callback_url.as_ref().map(Url::as_str),
    };

    let url = provider
        .token_url
        .clone()
        .ok_or_else(|| err!(Config("token_url", "Missing token URL in config")))?;

    self.request(
        (Some(provider), Some(session)),
        Method::POST,
        url,
        Some(query),
    )
    .await
    .and_then(|value| serde_json::from_value(value).map_err(Into::into))
    .log_err()
}

#[implement(Service)]
#[tracing::instrument(
    name = "request",
    level = "debug",
    ret(level = "trace"),
    skip(self, body)
)]
pub async fn request<Body>(
    &self,
    (provider, session): (Option<&Provider>, Option<&Session>),
    method: Method,
    url: Url,
    body: Option<Body>,
) -> Result<JsonValue>
where
    Body: Serialize,
{
    let mut request = self
        .services
        .client
        .oauth
        .request(method, url)
        .header(ACCEPT, "application/json");

    let body = body
        .map(|body| serde_html_form::to_string(body))
        .transpose()
        .map_err(|e| err!(SerdeSer("Failed to form-encode the request body: {e}")))?;

    if let Some(body) = body {
        request = request
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
    }

    if let Some(session) = session
        && let Some(access_token) = session.access_token.as_deref()
    {
        request = request.bearer_auth(access_token);
    }

    let limit = self.services.config.oidc.oidc_max_response_size;
    let http_response = request.send().await?.error_for_status()?;

    let body = read_response_capped(http_response, limit).await?;
    let response: JsonValue = serde_json::from_slice(&body)?;

    if let Some(object) = response.as_object()
        && let Some(error) = object.get("error").and_then(JsonValue::as_str)
    {
        let description = object
            .get("error_description")
            .and_then(JsonValue::as_str)
            .unwrap_or("(no description)");

        return Err!(Request(Forbidden(
            "Error from provider: {error}: {description}"
        )));
    }

    Ok(response)
}

#[inline]
pub fn unique_id((provider, session): (&Provider, &Session)) -> Result<String> {
    unique_id_parts((provider, session)).and_then(unique_id_iss_sub)
}

#[inline]
pub fn unique_id_sub((provider, sub): (&Provider, &str)) -> Result<String> {
    identity_issuer(provider)
        .ok_or_else(|| {
            err!(Config(
                "issuer_url",
                "issuer_url not found for this provider."
            ))
        })
        .map(|iss| (iss, sub))
        .and_then(unique_id_iss_sub)
}

#[inline]
pub fn unique_id_iss((iss, session): (&str, &Session)) -> Result<String> {
    unique_id_iss_parts((iss, session)).and_then(unique_id_iss_sub)
}

pub fn unique_id_iss_sub((iss, sub): (&str, &str)) -> Result<String> {
    Ok(b64encode.encode(sha256::delimited([iss, sub].iter())))
}

fn unique_id_parts<'a>(
    (provider, session): (&'a Provider, &'a Session),
) -> Result<(&'a str, &'a str)> {
    identity_issuer(provider)
        .ok_or_else(|| {
            err!(Config(
                "issuer_url",
                "issuer_url not found for this provider."
            ))
        })
        .and_then(|iss| unique_id_iss_parts((iss, session)))
}

fn identity_issuer(provider: &Provider) -> Option<&str> {
    match provider.brand.as_str() {
        "github" => Some("https://github.com/"),
        _ => provider.issuer_url.as_ref().map(Url::as_str),
    }
}

fn unique_id_iss_parts<'a>((iss, session): (&'a str, &'a Session)) -> Result<(&'a str, &'a str)> {
    session
        .user_info
        .as_ref()
        .map(|user_info| user_info.sub.as_str())
        .ok_or_else(|| err!(Request(NotFound("user_info not found for this session."))))
        .map(|sub| (iss, sub))
}

#[cfg(test)]
mod tests {
    use super::unique_id_iss_sub;

    #[test]
    fn the_identity_hash_is_stable() {
        let once = unique_id_iss_sub(("https://github.com/", "12345")).expect("hashes");
        let again = unique_id_iss_sub(("https://github.com/", "12345")).expect("hashes");

        assert_eq!(once, again);
    }

    #[test]
    fn the_identity_hash_cannot_be_resplit() {
        let left = unique_id_iss_sub(("https://a.example/", "bc")).expect("hashes");
        let right = unique_id_iss_sub(("https://a.example/b", "c")).expect("hashes");

        assert_ne!(left, right);
    }

    #[test]
    fn a_different_issuer_is_a_different_identity() {
        let github = unique_id_iss_sub(("https://github.com/", "12345")).expect("hashes");
        let gitlab = unique_id_iss_sub(("https://gitlab.com/", "12345")).expect("hashes");

        assert_ne!(
            github, gitlab,
            "the same subject at two providers is two people"
        );
    }
}
