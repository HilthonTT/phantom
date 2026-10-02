#[cfg(test)]
mod tests;

mod account_deactivate;
mod cross_signing_reset;
mod profile;
mod profile_saved;
mod session_end_confirm;
mod session_end_execute;
mod session_list;
mod session_view;

use axum::{
    extract::{Form, Request, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use http::{
    HeaderValue, Method, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY},
};
use phantom_core::{Err, Error, Result, err, html::escape as html_escape};
use phantom_service::Services;
use ruma::{MilliSecondsSinceUnixEpoch, OwnedDeviceId};
use url::Url;

use self::{
    account_deactivate::{account_deactivate_confirm_html, account_deactivate_execute_html},
    cross_signing_reset::{cross_signing_reset_confirm_html, cross_signing_reset_execute_html},
    profile::profile_html,
    profile_saved::profile_saved_html,
    session_end_confirm::session_end_confirm_html,
    session_end_execute::session_end_execute_html,
    session_list::sessions_list_html,
    session_view::session_view_html,
};
use super::{
    NativeChoice, consume_login_token, peek_login_token, query_error, should_serve_native,
    sso_redirect_url, url_encode,
};

pub(crate) static ACCOUNT_MANAGEMENT_ACTIONS_SUPPORTED: &[&str] = &[
    "org.matrix.profile",
    "org.matrix.devices_list",
    "org.matrix.device_view",
    "org.matrix.device_delete",
    "org.matrix.account_deactivate",
    "org.matrix.cross_signing_reset",
    "org.matrix.sessions_list",
    "org.matrix.session_view",
    "org.matrix.session_end",
];

/// Raw JS served at `/_phantom/oidc/account.js`.
/// Referenced via `<script src>` for CSP compatibility.
static ACCOUNT_JS: &str = include_str!("account.js");

/// Shared stylesheet served at `/_phantom/oidc/account.css`.
static ACCOUNT_CSS: &str = include_str!("account.css");

/// Common `<head>` contents. The script is deferred, so it can sit here and
/// still see the whole document.
pub(super) static ACCOUNT_HEAD: &str = r#"<meta charset="utf-8">
		<meta name="viewport" content="width=device-width, initial-scale=1">
		<meta name="color-scheme" content="light dark">
		<meta name="referrer" content="no-referrer">
		<meta name="robots" content="noindex, nofollow">
		<link rel="stylesheet" href="/_phantom/oidc/account.css">
		<script src="/_phantom/oidc/account.js" defer></script>"#;

/// Cache-control header value.
static ACCOUNT_CACHE_CONTROL: &str = "no-store";

#[derive(Debug, Default, serde::Deserialize)]
struct AccountQueryParams {
    action: Option<String>,
    device_id: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct AccountCallbackParams {
    action: Option<String>,
    device_id: Option<String>,
    #[serde(rename = "loginToken")]
    login_token: Option<String>,
    displayname: Option<String>,
}

pub(crate) async fn get_account_route(
    State(services): State<crate::router::State>,
    request: Request,
) -> impl IntoResponse {
    let params: AccountQueryParams =
        match serde_html_form::from_str(request.uri().query().unwrap_or_default()) {
            Err(e) => return account_error_response(&query_error(&e)),
            Ok(params) => params,
        };

    let action = params
        .action
        .as_deref()
        .unwrap_or("org.matrix.sessions_list");

    let device_id = params.device_id.as_deref().unwrap_or_default();

    match account_auth_redirect(&services, action, device_id) {
        Ok(response) => response,
        Err(e) => account_error_response(&e),
    }
}

fn account_auth_redirect(services: &Services, action: &str, device_id: &str) -> Result<Response> {
    validate_account_action(action)?;

    let idp_id = services.oauth.providers.get_default_id();
    let serve_native = should_serve_native(NativeChoice {
        native_enabled: services.config.oidc.oidc_native_auth,
        has_default_idp: idp_id.is_some(),
    });

    if serve_native {
        account_native_redirect(services, action, device_id)
    } else {
        account_sso_redirect(services, action, device_id, idp_id.as_deref())
    }
}

fn account_native_redirect(services: &Services, action: &str, device_id: &str) -> Result<Response> {
    let issuer = services.oauth.get_server()?.issuer_url()?;
    let base = issuer.trim_end_matches('/');

    let native_url = Url::parse_with_params(
        &format!("{base}/_phantom/oidc/native"),
        [("action", action), ("device_id", device_id)],
    )
    .map_err(|_| err!(Request(InvalidParam("Failed to build native login URL"))))?;

    Ok(account_redirect_response(Redirect::temporary(
        native_url.as_str(),
    )))
}

fn account_sso_redirect(
    services: &Services,
    action: &str,
    device_id: &str,
    idp_id: Option<&str>,
) -> Result<Response> {
    let idp_id = idp_id.ok_or_else(|| {
        err!(Config(
            "identity_provider",
            "No identity provider configured"
        ))
    })?;

    let issuer = services.oauth.get_server()?.issuer_url()?;
    let base = issuer.trim_end_matches('/');

    let callback_url = Url::parse_with_params(
        &format!("{base}/_phantom/oidc/account_callback"),
        [("action", action), ("device_id", device_id)],
    )
    .map_err(|_| err!(error!("Failed to build account callback URL")))?;

    let sso_url = sso_redirect_url(base, idp_id, &callback_url)?;

    Ok(account_redirect_response(Redirect::temporary(
        sso_url.as_str(),
    )))
}

pub(crate) async fn get_account_callback_route(
    State(services): State<crate::router::State>,
    request: Request,
) -> impl IntoResponse {
    let params: AccountCallbackParams =
        match serde_html_form::from_str(request.uri().query().unwrap_or_default()) {
            Err(e) => return account_error_response(&query_error(&e)),
            Ok(params) => params,
        };

    match handle_account_callback(&services, Method::GET, params).await {
        Ok(html) => account_html_response(StatusCode::OK, html),
        Err(e) => account_error_response(&e),
    }
}

pub(crate) async fn post_account_callback_route(
    State(services): State<crate::router::State>,
    Form(body): Form<AccountCallbackParams>,
) -> impl IntoResponse {
    match handle_account_callback(&services, Method::POST, body).await {
        Ok(html) => account_html_response(StatusCode::OK, html),
        Err(e) => account_error_response(&e),
    }
}

// no-cache: revalidate on every request so a server update takes effect
// immediately
pub(crate) async fn account_js_route() -> impl IntoResponse {
    let content_type = (CONTENT_TYPE, "application/javascript; charset=utf-8");
    let cache_control = (CACHE_CONTROL, "no-cache");

    ([content_type, cache_control], ACCOUNT_JS)
}

pub(crate) async fn account_css_route() -> impl IntoResponse {
    let content_type = (CONTENT_TYPE, "text/css; charset=utf-8");
    let cache_control = (CACHE_CONTROL, "no-cache");

    ([content_type, cache_control], ACCOUNT_CSS)
}

async fn handle_account_callback(
    services: &Services,
    method: Method,
    params: AccountCallbackParams,
) -> Result<String> {
    let login_token = params.login_token.as_deref();

    let fallback_action = (method == Method::GET).then_some("org.matrix.sessions_list");

    let action = params
        .action
        .as_deref()
        .or(fallback_action)
        .unwrap_or_default();

    // Validations before consuming the token so that an invalid action does not
    // burn the user's single-use login_token needlessly.
    services.oauth.get_server()?;

    if !services.config.oidc.oidc_native_auth && services.oauth.providers.get_default_id().is_none()
    {
        return Err!(Config(
            "identity_provider",
            "No identity provider or native authentication configured"
        ));
    }

    validate_account_action(action)?;

    // MSC4191 stable action names dispatch through the prototype aliases.
    let action = normalize_account_action(action);

    // Read-only pages consume the token immediately. Pages with a POST
    // confirmation step peek at the token so it can be embedded in the form and
    // consumed only when the user confirms the action, which avoids minting a
    // second short-lived token on every GET.
    let user_id = match action {
        "org.matrix.sessions_list" => consume_login_token(services, login_token).await?,
        _ if method == Method::POST => consume_login_token(services, login_token).await?,
        _ if method == Method::GET => peek_login_token(services, login_token).await?,
        _ => {
            return Err!(Request(Unrecognized(
                "Unsupported account management method"
            )));
        }
    };

    let login_token = login_token.unwrap_or_default();
    let device_id = params.device_id.as_deref().unwrap_or_default();

    match action {
        "org.matrix.sessions_list" if method == Method::GET => {
            sessions_list_html(services, &user_id).await
        }

        "org.matrix.profile" if method == Method::GET => {
            profile_html(services, &user_id, login_token).await
        }

        "org.matrix.profile" if method == Method::POST => {
            // Strip control characters and keep at most 255 code points.
            let displayname: String = params
                .displayname
                .as_deref()
                .unwrap_or("")
                .trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(255)
                .collect();

            let displayname = (!displayname.is_empty()).then_some(displayname);

            services
                .profile
                .set_displayname(&user_id, displayname.clone());

            Ok(profile_saved_html(&user_id, displayname.as_deref()))
        }

        "org.matrix.session_view" if method == Method::GET => {
            session_view_html(services, &user_id, device_id, login_token).await
        }

        "org.matrix.session_end" if method == Method::POST => {
            session_end_execute_html(services, &user_id, device_id).await
        }

        "org.matrix.session_end" if method == Method::GET => {
            // Authenticate first (peek), then show a POST confirmation form.
            // Deletion happens only on POST, so a GET cannot be forged into one.
            if device_id.is_empty() {
                return Err!(Request(InvalidParam("device_id is required")));
            }

            let device_id: OwnedDeviceId = device_id.into();
            if !services.users.device_exists(&user_id, &device_id).await {
                return Err!(Request(NotFound("Session not found")));
            }

            Ok(session_end_confirm_html(
                &user_id,
                device_id.as_str(),
                login_token,
            ))
        }

        "org.matrix.account_deactivate" if method == Method::POST => {
            account_deactivate_execute_html(services, &user_id).await
        }

        "org.matrix.account_deactivate" if method == Method::GET => {
            Ok(account_deactivate_confirm_html(&user_id, login_token))
        }

        "org.matrix.cross_signing_reset" if method == Method::POST => {
            cross_signing_reset_execute_html(services, &user_id).await
        }

        "org.matrix.cross_signing_reset" if method == Method::GET => {
            Ok(cross_signing_reset_confirm_html(&user_id, login_token))
        }

        _ => Err!(Request(InvalidParam(
            "Unsupported account management action"
        ))),
    }
}

pub(super) fn account_redirect_response(redirect: Redirect) -> Response {
    let mut response = redirect.into_response();
    let headers = response.headers_mut();

    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static(ACCOUNT_CACHE_CONTROL),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));

    response
}

// Prevent the login token in the callback URL from leaking via the Referer
// header to any embedded resources.
pub(super) fn account_html_response(status: StatusCode, html: String) -> Response {
    let headers = [
        (CACHE_CONTROL, ACCOUNT_CACHE_CONTROL),
        (REFERRER_POLICY, "no-referrer"),
    ];

    (status, headers, Html(html)).into_response()
}

pub(super) fn account_error_response(error: &Error) -> Response {
    let msg = error.sanitized_message();
    let code = error.status_code();

    account_html_response(code, account_error_page(&msg))
}

fn account_error_page(message: &str) -> String {
    let msg = html_escape(message);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Error</title>
	</head>
	<body>
		<main>
			<h1 class="err">Error</h1>
			<p>{msg}</p>
			<nav class="nav">
				<a href="/_phantom/oidc/account">Return to account management</a>
			</nav>
		</main>
	</body>
</html>"#
    )
}

fn validate_account_action(action: &str) -> Result {
    if !ACCOUNT_MANAGEMENT_ACTIONS_SUPPORTED.contains(&action) {
        return Err!(Request(InvalidParam(
            "Unsupported account management action"
        )));
    }

    Ok(())
}

fn normalize_account_action(action: &str) -> &str {
    match action {
        "org.matrix.devices_list" => "org.matrix.sessions_list",
        "org.matrix.device_view" => "org.matrix.session_view",
        "org.matrix.device_delete" => "org.matrix.session_end",
        other => other,
    }
}

/// Renders a last-seen timestamp; `account.js` localises the `<time>` text.
fn ts_cell(ts: Option<MilliSecondsSinceUnixEpoch>) -> String {
    match ts.map(|ts| u64::from(ts.as_secs())) {
        None | Some(0) => "—".to_owned(),
        Some(ts_secs) => format!(r#"<time data-ts="{ts_secs}">—</time>"#),
    }
}
