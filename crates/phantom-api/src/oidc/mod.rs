pub mod account;
mod auth_issuer;
mod auth_metadata;
mod authorize;
mod complete;
pub mod device;
mod jwks;
mod native;
mod registration;
mod revoke;
mod token;
mod userinfo;

#[cfg(test)]
mod tests;

use std::fmt::Write;

use axum::{
    Json, Router,
    body::Body,
    response::IntoResponse,
    routing::{get, post},
};
use http::{Response, StatusCode};
use phantom_core::{Result, err};
use phantom_service::Services;
use ruma::OwnedUserId;
use serde_json::json;
use url::Url;

use crate::router::State;

const OIDC_REQ_ID_LENGTH: usize = 32;

#[derive(Clone, Copy)]
struct NativeChoice {
    native_enabled: bool,
    has_default_idp: bool,
}

/// Routes of the OIDC server for next-gen auth (MSC2964/MSC2965/MSC2966/
/// MSC2967/MSC4191/MSC4254). Each handler refuses on its own when the server is
/// not configured, so they are registered unconditionally.
pub fn register(router: Router<State>) -> Router<State> {
    router
        .route(
            "/_phantom/oidc/registration",
            post(registration::registration_route),
        )
        .route("/_phantom/oidc/authorize", get(authorize::authorize_route))
        .route(
            "/_phantom/oidc/_complete",
            get(complete::complete_route).post(complete::post_complete_route),
        )
        .route(
            "/_phantom/oidc/native",
            get(native::native_get_route).post(native::native_submit_route),
        )
        .route("/_phantom/oidc/token", post(token::token_route))
        .route(
            "/_phantom/oidc/device_authorization",
            post(device::device_authorization_route),
        )
        .route("/_phantom/oidc/device", get(device::get_device_route))
        .route(
            "/_phantom/oidc/device_callback",
            get(device::get_device_callback_route).post(device::post_device_callback_route),
        )
        .route("/_phantom/oidc/revoke", post(revoke::revoke_route))
        .route("/_phantom/oidc/jwks", get(jwks::jwks_route))
        .route(
            "/_phantom/oidc/userinfo",
            get(userinfo::userinfo_route).post(userinfo::userinfo_route),
        )
        .route("/_phantom/oidc/account.js", get(account::account_js_route))
        .route(
            "/_phantom/oidc/account.css",
            get(account::account_css_route),
        )
        .route(
            "/_phantom/oidc/account_callback",
            get(account::get_account_callback_route).post(account::post_account_callback_route),
        )
        .route("/_phantom/oidc/account", get(account::get_account_route))
        .route(
            "/_matrix/client/v1/auth_issuer",
            get(auth_issuer::auth_issuer_route),
        )
        .route(
            "/_matrix/client/v1/auth_metadata",
            get(auth_metadata::openid_configuration_route),
        )
        .route(
            "/_matrix/client/unstable/org.matrix.msc2965/auth_issuer",
            get(auth_issuer::auth_issuer_route),
        )
        .route(
            "/_matrix/client/unstable/org.matrix.msc2965/auth_metadata",
            get(auth_metadata::openid_configuration_route),
        )
        .route(
            "/.well-known/openid-configuration",
            get(auth_metadata::openid_configuration_route),
        )
}

pub(crate) fn url_encode(s: &str) -> String {
    s.bytes()
        .fold(String::with_capacity(s.len()), |mut out, b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                out.push(b.into());
            } else {
                write!(&mut out, "%{b:02X}").ok();
            }

            out
        })
}

fn query_error(e: &impl std::fmt::Display) -> phantom_core::Error {
    err!(Request(InvalidParam(
        "Failed to read query parameters: {e}"
    )))
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response<Body> {
    let body = json!({
        "error": error,
        "error_description": description,
    });

    (status, Json(body)).into_response()
}

async fn consume_login_token(services: &Services, token: Option<&str>) -> Result<OwnedUserId> {
    let token = token.ok_or_else(|| err!(Request(Forbidden("Missing login token"))))?;

    services
        .users
        .find_from_login_token(token)
        .await
        .map_err(|_| err!(Request(Forbidden("Invalid or expired login token"))))
}

/// Verify a login token without consuming it; it is consumed later when the
/// confirmation form is submitted.
async fn peek_login_token(services: &Services, token: Option<&str>) -> Result<OwnedUserId> {
    let token = token.ok_or_else(|| err!(Request(Forbidden("Missing login token"))))?;

    services
        .users
        .peek_login_token(token)
        .await
        .map_err(|_| err!(Request(Forbidden("Invalid or expired login token"))))
}

/// Whether a redirect URI is covered by the operator's redirect allowlist.
///
/// A URI carrying a host matches an allowlist entry naming that host. A
/// private-use scheme carries no host at all (RFC 8252 §7.1, as in
/// `io.element.android:/callback`), so it matches an entry naming the scheme
/// instead, which is what lets one list cover both a web and a mobile client.
/// Either way the comparison ignores case.
fn redirect_allowlisted(allowed: &[String], uri: &str) -> bool {
    Url::parse(uri).is_ok_and(|url| {
        let name = url.host_str().unwrap_or_else(|| url.scheme());

        allowed.iter().any(|entry| entry.eq_ignore_ascii_case(name))
    })
}

/// Whether a flow with no provider chooser serves the native page.
///
/// Native applies only when native auth is enabled and no default provider is
/// configured; every other flow goes through single sign-on.
fn should_serve_native(
    NativeChoice {
        native_enabled,
        has_default_idp,
    }: NativeChoice,
) -> bool {
    native_enabled && !has_default_idp
}

/// Build the upstream SSO redirect URL for a pending authorization request.
///
/// The provider hands the browser back to the completion route carrying the
/// request id, where the authorization code is minted. A trailing slash on the
/// issuer is ignored.
fn authorization_sso_url(issuer: &str, idp_id: &str, req_id: &str) -> Result<Url> {
    let base = issuer.trim_end_matches('/');
    let complete = format!("{base}/_phantom/oidc/_complete");
    let callback = Url::parse_with_params(&complete, [("oidc_req_id", req_id)])
        .map_err(|_| err!(error!("Failed to build complete URL")))?;

    sso_redirect_url(base, idp_id, &callback)
}

fn sso_redirect_url(base: &str, idp_id: &str, callback: &Url) -> Result<Url> {
    let idp_id_enc = url_encode(idp_id);
    let mut sso_url = Url::parse(&format!(
        "{base}/_matrix/client/v3/login/sso/redirect/{idp_id_enc}"
    ))
    .map_err(|_| err!(error!("Failed to build SSO URL")))?;

    sso_url
        .query_pairs_mut()
        .append_pair("redirectUrl", callback.as_str());

    Ok(sso_url)
}
