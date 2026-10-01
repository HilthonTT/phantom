pub mod account;
pub mod device;

use std::fmt::Write;

use axum::{Json, body::Body, response::IntoResponse};
use http::{Response, StatusCode};
use phantom_core::{Result, err};
use phantom_service::Services;
use ruma::OwnedUserId;
use serde_json::json;
use url::Url;

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

/// Whether to serve the native login page rather than redirecting to the
/// default identity provider.
fn should_serve_native(native_enabled: bool, has_default_idp: bool, wants_create: bool) -> bool {
    native_enabled && (!has_default_idp || wants_create)
}

fn sso_redirect_url(base: &str, idp_id: &str, callback: &Url) -> Result<Url> {
    let idp_id_enc = url_encode(idp_id);
    let mut sso_url =
        Url::parse(&format!("{base}/_matrix/client/v3/login/sso/redirect/{idp_id_enc}"))
            .map_err(|_| err!(error!("Failed to build SSO URL")))?;

    sso_url
        .query_pairs_mut()
        .append_pair("redirectUrl", callback.as_str());

    Ok(sso_url)
}
