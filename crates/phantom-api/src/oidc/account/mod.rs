mod account_deactivate;
mod cross_signing_reset;
mod profile;
mod profile_saved;

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
