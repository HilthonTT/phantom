use phantom_core::html::escape as html_escape;
use ruma::UserId;

use super::ACCOUNT_HEAD;

pub(super) fn profile_saved_html(user_id: &UserId, displayname: Option<&str>) -> String {
    let uid = html_escape(user_id.as_str());
    let message = match displayname.filter(|dn| !dn.is_empty()) {
        Some(dn) => format!(
            "Display name for <strong>{uid}</strong> is now <strong>{}</strong>.",
            html_escape(dn)
        ),
        None => format!("Display name for <strong>{uid}</strong> was removed."),
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Profile saved</title>
	</head>
	<body>
		<main>
			<h1 class="ok">&check; Profile saved</h1>
			<p role="status">{message}</p>
			<nav class="nav">
				<a href="/_phantom/oidc/account?action=org.matrix.profile">Edit profile</a>
				<a href="/_phantom/oidc/account?action=org.matrix.sessions_list">
					&larr; Back to sessions
				</a>
			</nav>
		</main>
	</body>
</html>"#
    )
}
