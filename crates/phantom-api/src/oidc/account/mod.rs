mod account_deactivate;
mod cross_signing_reset;
mod profile;
mod profile_saved;
mod session_end_confirm;
mod session_end_execute;
mod session_list;
mod session_view;

use axum::response::{Html, IntoResponse, Response};
use http::{
    StatusCode,
    header::{CACHE_CONTROL, REFERRER_POLICY},
};
use phantom_core::{Err, Error, Result, html::escape as html_escape};
use ruma::MilliSecondsSinceUnixEpoch;

use super::url_encode;

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
				<h1 class="err">Error</h1>
				<p>{msg}</p>
				<div class="nav">
					<a href="/_phantom/oidc/account">
						Return to account management
					</a>
				</div>
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
