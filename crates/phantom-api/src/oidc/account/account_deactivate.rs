use const_str::format as const_format;
use phantom_core::{Result, html::escape as html_escape, info};
use phantom_service::Services;
use ruma::UserId;

use super::{ACCOUNT_HEAD, url_encode};

pub(super) async fn account_deactivate_confirm_html(
    user_id: &UserId,
    login_token: &str,
) -> Result<String> {
    let uid = html_escape(user_id.as_str());
    let tok = html_escape(login_token);
    let tok_enc = url_encode(login_token);

    Ok(CONFIRM_HTML
        .replace("{uid}", &uid)
        .replace("{tok}", &tok)
        .replace("{tok_enc}", &tok_enc))
}

pub(super) async fn account_deactivate_execute_html(
    services: &Services,
    user_id: &UserId,
) -> Result<String> {
    services.users.deactivate_account(user_id).await?;

    info!(?user_id, "Account deactivated via account management page");

    Ok(EXECUTE_HTML.replace("{uid}", &html_escape(user_id.as_str())))
}

static CONFIRM_HTML: &str = const_format!(
    r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Deactivate account</title>
	</head>
	<body>
		<main>
			<h1>Deactivate account</h1>
			<p class="meta">Signed in as <strong>{{uid}}</strong></p>
			<div class="callout danger" role="note">
				<p><strong>This is permanent and cannot be undone.</strong></p>
				<p>
					All of your sessions will be signed out and you will not be able to
					sign in to this account again.
				</p>
			</div>
			<form method="POST" action="/_phantom/oidc/account_callback">
				<input type="hidden" name="action" value="org.matrix.account_deactivate">
				<input type="hidden" name="loginToken" value="{{tok}}">
				<label class="check">
					<input type="checkbox" required>
					I understand that <strong>{{uid}}</strong> will be deactivated for good
				</label>
				<div class="submit-row">
					<button type="submit" class="danger">Deactivate account</button>
					<a href="/_phantom/oidc/account_callback?action=org.matrix.sessions_list&amp;loginToken={{tok_enc}}">
						Cancel
					</a>
				</div>
			</form>
		</main>
	</body>
</html>
"#
);

static EXECUTE_HTML: &str = const_format!(
    r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Account deactivated</title>
	</head>
	<body>
		<main>
			<h1>Account deactivated</h1>
			<p role="status">
				<strong>{{uid}}</strong> has been deactivated and all of its sessions
				have been signed out. You can close this page.
			</p>
		</main>
	</body>
</html>
"#
);
