use const_str::format as const_format;
use phantom_core::{Err, Result, html::escape as html_escape, info};
use phantom_service::Services;
use ruma::{OwnedDeviceId, UserId};

use super::ACCOUNT_HEAD;

pub(super) async fn session_end_execute_html(
    services: &Services,
    user_id: &UserId,
    device_id: &str,
) -> Result<String> {
    if device_id.is_empty() {
        return Err!(Request(InvalidParam("device_id is required")));
    }

    let device_id_owned: OwnedDeviceId = device_id.into();
    if services
        .users
        .get_device_metadata(user_id, &device_id_owned)
        .await
        .is_err()
    {
        return Err!(Request(NotFound("Session not found")));
    }

    services
        .users
        .remove_device(user_id, &device_id_owned)
        .await;

    info!(
        ?user_id,
        ?device_id_owned,
        "Session signed out via account management page"
    );

    Ok(PAGE_HTML
        .replace("{did}", &html_escape(device_id_owned.as_str()))
        .replace("{uid}", &html_escape(user_id.as_str())))
}

static PAGE_HTML: &str = const_format!(
    r#"
<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Session Signed Out</title>
	</head>
	<body>
		<h1 class="ok">Session Signed Out</h1>
		<p>
			Session <code>{{did}}</code> for <strong>{{uid}}</strong> has been signed out.
		</p>
		<div class="nav">
			<a href="/_phantom/oidc/account?action=org.matrix.sessions_list">
				Back to sessions
			</a>
		</div>
	</body>
</html>"#
);
