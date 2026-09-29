use phantom_core::html::escape as html_escape;
use ruma::UserId;

use super::{ACCOUNT_HEAD, url_encode};

pub(super) fn session_end_confirm_html(
    user_id: &UserId,
    device_id: &str,
    login_token: &str,
) -> String {
    let uid = html_escape(user_id.as_str());
    let did = html_escape(device_id);
    let tok = html_escape(login_token);

    // url_encode for use in the Cancel href query parameter.
    let did_enc = url_encode(device_id);
    let tok_enc = url_encode(login_token);

    // device_id comes straight from the request; format! substitutes in one
    // pass, so it can't smuggle in a placeholder such as the login token.
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Sign Out Session</title>
	</head>
	<body>
		<main>
			<h1>Sign Out Session</h1>
			<p class="meta">Signed in as <strong>{uid}</strong></p>
			<p class="warn">
				Sign out session <code>{did}</code>?
				This will immediately invalidate its access token.
			</p>
			<form method="POST" action="/_phantom/oidc/account_callback">
				<input type="hidden" name="action" value="org.matrix.session_end">
				<input type="hidden" name="device_id" value="{did}">
				<input type="hidden" name="loginToken" value="{tok}">
				<div class="submit-row">
					<button type="submit" class="danger">Sign out</button>
					<a href="/_phantom/oidc/account_callback?action=org.matrix.session_view&amp;device_id={did_enc}&amp;loginToken={tok_enc}">
						Cancel
					</a>
				</div>
			</form>
		</main>
	</body>
</html>"#
    )
}
