use const_str::format as const_format;
use phantom_core::{Result, html::escape as html_escape, info};
use phantom_service::Services;
use ruma::UserId;

use super::{ACCOUNT_HEAD, url_encode};

pub(super) async fn cross_signing_reset_confirm_html(
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

pub(super) async fn cross_signing_reset_execute_html(
    services: &Services,
    user_id: &UserId,
) -> Result<String> {
    services.users.allow_cross_signing_replacement(user_id);

    info!(
        ?user_id,
        "Cross-signing reset approved via account management page"
    );

    Ok(EXECUTE_HTML.replace("{uid}", &html_escape(user_id.as_str())))
}

static CONFIRM_HTML: &str = const_format!(
    r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Reset cross-signing</title>
	</head>
	<body>
		<main>
			<h1>Reset cross-signing</h1>
			<p class="meta">Signed in as <strong>{{uid}}</strong></p>
			<div class="callout warn" role="note">
				<p><strong>Your other sessions and contacts will need to verify you again.</strong></p>
				<p>
					After you approve, your Matrix client has ten minutes to upload a new
					cross-signing identity. Only do this if you've lost access to your
					recovery key or every verified session.
				</p>
			</div>
			<form method="POST" action="/_phantom/oidc/account_callback">
				<input type="hidden" name="action" value="org.matrix.cross_signing_reset">
				<input type="hidden" name="loginToken" value="{{tok}}">
				<div class="submit-row">
					<button type="submit" class="danger">Approve reset</button>
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
		<title>Cross-signing reset approved</title>
	</head>
	<body>
		<main>
			<h1 class="ok">&check; Cross-signing reset approved</h1>
			<p role="status">
				Go back to your Matrix client and finish setting up a new
				cross-signing identity for <strong>{{uid}}</strong>.
			</p>
			<p class="meta">This approval expires in ten minutes.</p>
			<nav class="nav">
				<a href="/_phantom/oidc/account?action=org.matrix.sessions_list">
					&larr; Back to sessions
				</a>
			</nav>
		</main>
	</body>
</html>
"#
);
