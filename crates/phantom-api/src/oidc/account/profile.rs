use phantom_core::{Result, html::escape as html_escape};
use phantom_service::Services;
use ruma::UserId;

use super::ACCOUNT_HEAD;

pub(super) async fn profile_html(
    services: &Services,
    user_id: &UserId,
    login_token: &str,
) -> Result<String> {
    let server = services.config.server_name.as_str();

    let displayname = services
        .profile
        .displayname(user_id)
        .await
        .unwrap_or_default();

    let avatar_field = services
        .profile
        .avatar_url(user_id)
        .await
        .ok()
        .map(|avatar_url| {
            let avatar_url = html_escape(avatar_url.as_str());
            format!(
                r#"<p class="meta">
					Avatar: <code>{avatar_url}</code><br>
					Use your Matrix client to change it.
				</p>"#
            )
        })
        .unwrap_or_default();

    let server = html_escape(server);
    let uid = html_escape(user_id.as_str());
    let tok = html_escape(login_token);
    let dn = html_escape(&displayname);

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Profile · {server}</title>
	</head>
	<body>
		<main>
			<h1>Profile</h1>
			<p class="meta">
				Signed in as <strong>{uid}</strong> on <strong>{server}</strong>
			</p>
			<form method="POST" action="/_phantom/oidc/account_callback">
				<input type="hidden" name="action" value="org.matrix.profile">
				<input type="hidden" name="loginToken" value="{tok}">
				<label for="displayname">Display name</label>
				<input
					type="text"
					id="displayname"
					name="displayname"
					value="{dn}"
					maxlength="255"
					autocomplete="nickname"
					spellcheck="false"
				>
				{avatar_field}
				<div class="submit-row">
					<button type="submit">Save</button>
				</div>
			</form>
			<nav class="nav">
				<a href="/_phantom/oidc/account?action=org.matrix.sessions_list">
					&larr; Back to sessions
				</a>
			</nav>
		</main>
	</body>
</html>"#
    ))
}
