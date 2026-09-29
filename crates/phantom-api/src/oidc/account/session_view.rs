use phantom_core::{Err, Result, err, html::escape as html_escape};
use phantom_service::Services;
use ruma::{DeviceId, UserId};

use super::{ACCOUNT_HEAD, ts_cell, url_encode};

pub(super) async fn session_view_html(
    services: &Services,
    user_id: &UserId,
    device_id: &str,
    login_token: &str,
) -> Result<String> {
    if device_id.is_empty() {
        return Err!(Request(InvalidParam("device_id is required")));
    }

    let device = services
        .users
        .get_device_metadata(user_id, <&DeviceId>::from(device_id))
        .await
        .map_err(|_| err!(Request(NotFound("Session not found"))))?;

    let uid = html_escape(user_id.as_str());
    let name = html_escape(device.display_name.as_deref().unwrap_or("Unknown device"));
    let id = html_escape(device.device_id.as_str());
    let id_enc = url_encode(device.device_id.as_str());
    let ip = html_escape(device.last_seen_ip.as_deref().unwrap_or("—"));
    let ts = ts_cell(device.last_seen_ts);

    // url_encode for use in the sign-out href query parameter.
    let tok_enc = url_encode(login_token);

    // Link directly to account_callback (skips SSO) using the peeked login_token
    // so the user doesn't have to re-authenticate just to sign out a session.
    // format! substitutes in one pass, so user-controlled values (display
    // name, device ID) can't be mistaken for a placeholder.
    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Session: {name}</title>
	</head>
	<body>
		<h1>Session Details</h1>
		<p>
			Signed in as <strong>{uid}</strong>.
		</p>
		<dl>
			<dt>Name</dt><dd>{name}</dd>
			<dt>Device ID</dt><dd><code>{id}</code></dd>
			<dt>Last seen IP</dt><dd>{ip}</dd>
			<dt>Last seen</dt><dd>{ts}</dd>
		</dl>
		<div class="actions">
			<a href="/_phantom/oidc/account?action=org.matrix.sessions_list">
				Back to sessions
			</a>
			<a
				href="/_phantom/oidc/account_callback?action=org.matrix.session_end&device_id={id_enc}&loginToken={tok_enc}"
				class="err"
			>
				Sign out this session
			</a>
		</div>
	</body>
</html>"#
    ))
}
