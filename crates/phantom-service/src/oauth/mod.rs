//! Logging in through somebody else.
//!
//! The service has two halves that meet in the middle. Outwards, it is an
//! OAuth 2 client: it discovers the identity providers an operator configured
//! ([`providers`]), sends users off to them, and holds what comes back
//! ([`sessions`]). Inwards, it is an OpenID Connect provider of its own
//! ([`server`]) — the one a Matrix client speaking next-gen auth asks for a
//! token. Only the outward half is always there; the server is built where the
//! configuration supports it and is `None` otherwise.
//!
//! What this module does *not* do is decide who a provider's answer makes
//! someone. Mapping claims onto a Matrix user, registering an account for a
//! new identity, and every HTTP endpoint involved all sit above this: what is
//! here is the provider configuration, the network calls to it, the persisted
//! authorizations, and the identity key those authorizations are found by.
//!
//! # Identity
//!
//! An identity is the pair of a provider's issuer and the subject it gave the
//! user, hashed — see [`unique_id`]. Neither half is enough on its own: two
//! providers can hand out the same subject, and the same provider reached at
//! two issuers is, as far as anyone can tell, two providers. The pair is what
//! `oauthuniqid_oauthid` is keyed on, and it is why `issuer_url` is the one
//! provider option that must never change: change it and every account bound
//! through it becomes unreachable, and the next login registers a new one.

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

/// A token bucket per client address: when it was last drawn from, and what is
/// left in it.
type Ratelimiter = Mutex<HashMap<IpAddr, (Instant, f64)>>;

pub struct Service {
    services: Services,

    pub providers: Arc<Providers>,
    pub sessions: Arc<Sessions>,

    /// This server's own OIDC provider, where the configuration supports one.
    /// See [`server::Server::build`].
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

/// This server's own OIDC provider, or a "not implemented here" error.
///
/// Every endpoint of the OIDC server reaches it through this, so a deployment
/// that did not configure one answers those endpoints as unrecognised rather
/// than as broken.
#[implement(Service)]
#[inline]
pub fn get_server(&self) -> Result<&Server> {
    self.server
        .as_deref()
        .ok_or_else(|| err!(Request(Unrecognized("The OIDC server is not configured"))))
}

/// Cap on the size of a rate-limit table. Past it, the addresses whose buckets
/// have refilled are dropped — a full bucket is indistinguishable from one
/// that was never made — so a spray of source addresses cannot grow the table
/// without bound.
const RATELIMIT_MAP_CAP: usize = 1 << 16;

/// The throttle on the device user-code endpoints, which is not configurable.
///
/// A `user_code` is short by design (RFC 8628 §6.1), so §5.1 requires the
/// guesses at it be bounded whatever the `oidc_rc_*` options say. The burst is
/// generous, because it only ever has to cover the one code a real person is
/// typing.
const DEVICE_RC_PER_SECOND: f64 = 1.0;
const DEVICE_RC_BURST: f64 = 60.0;

/// The per-address throttle on the OIDC endpoints.
///
/// Does nothing unless both `oidc_rc_per_second` and `oidc_rc_burst_count` are
/// set, since these endpoints are reached through a browser where a redirect
/// chain looks a great deal like a burst.
#[implement(Service)]
pub fn check_rate_limit(&self, client: IpAddr) -> Result {
    let config = &self.services.config;
    let rate = f64::from(config.oidc_rc_per_second);
    let burst = f64::from(config.oidc_rc_burst_count);

    if rate <= 0.0 || burst <= 0.0 {
        return Ok(());
    }

    check_bucket(&self.ratelimiter, client, rate, burst)
}

/// The always-on throttle on the device user-code endpoints (RFC 8628 §5.1),
/// which the `oidc_rc_*` options do not turn off.
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

/// Deletes every session a user has.
///
/// For debugging and for an operator who knows what they are doing: the
/// sessions are what tie a provider identity to this account, so deleting them
/// means the next login through that provider finds nothing and registers
/// somebody new.
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

/// Revokes every token a user holds at the providers that issued them.
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

/// Every authorization a user holds, with the provider it is at.
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

/// Asks a provider who this session belongs to.
///
/// The session's access token has to still be good; a provider answers this
/// for the bearer, not for a name.
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

/// Asks a provider what a session's access token is, and whether it is still
/// live.
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

/// Tells a provider to revoke a session's token.
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

/// Exchanges an authorization code for the provider's tokens.
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

/// One request to a provider, on the OAuth client.
///
/// Deliberately unopinionated about where it is going: the URL was resolved by
/// discovery, and discovery is what says where a provider's endpoints are.
/// What this adds is what every one of those requests needs — a form-encoded
/// body, the session's bearer token, a size cap on the response — and the one
/// piece of OAuth 2 in the answer that is not HTTP: an `error` property in a
/// `200` body, which is how providers report a refusal.
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

    let limit = self.services.config.oidc_max_response_size;
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

/// The identity a session represents: the provider's issuer and subject,
/// hashed.
#[inline]
pub fn unique_id((provider, session): (&Provider, &Session)) -> Result<String> {
    unique_id_parts((provider, session)).and_then(unique_id_iss_sub)
}

/// [`unique_id`] from a subject directly, for a caller that has the claims but
/// not a session to hold them.
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

/// [`unique_id`] against an issuer given directly rather than taken from a
/// provider's configuration.
#[inline]
pub fn unique_id_iss((iss, session): (&str, &Session)) -> Result<String> {
    unique_id_iss_parts((iss, session)).and_then(unique_id_iss_sub)
}

/// The identity hash of an issuer and subject.
///
/// Delimited rather than concatenated, so that no pair of issuer and subject
/// can be split differently into another pair that hashes the same.
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

/// The issuer string an identity is hashed against.
///
/// Usually the configured `issuer_url`, but pinned per brand for the providers
/// whose published issuer has moved under us: the hash is what an account is
/// found by, so following such a change would orphan every account bound
/// before it.
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

    /// The identity key is what an account is found by, so the same pair has
    /// to hash the same every time — including across restarts, which is why
    /// it is a hash of the two strings and not of anything else.
    #[test]
    fn the_identity_hash_is_stable() {
        let once = unique_id_iss_sub(("https://github.com/", "12345")).expect("hashes");
        let again = unique_id_iss_sub(("https://github.com/", "12345")).expect("hashes");

        assert_eq!(once, again);
    }

    /// Concatenating the two would let one pair be re-split into another: an
    /// issuer of `https://a.example/` with subject `bc` would hash the same as
    /// `https://a.example/b` with subject `c`, and either identity could then
    /// claim the other's account.
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
