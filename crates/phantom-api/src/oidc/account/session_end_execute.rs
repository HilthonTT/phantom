use phantom_core::{Err, Result, err, html::escape as html_escape, info};
use phantom_service::Services;
use ruma::{DeviceId, UserId};

use super::ACCOUNT_HEAD;

pub(super) async fn session_end_execute_html(
    services: &Services,
    user_id: &UserId,
    device_id: &str,
) -> Result<String> {
    if device_id.is_empty() {
        return Err!(Request(InvalidParam("device_id is required")));
    }

    let device_id = <&DeviceId>::from(device_id);

    // Only sign out sessions that belong to this user.
    services
        .users
        .get_device_metadata(user_id, device_id)
        .await
        .map_err(|_| err!(Request(NotFound("Session not found"))))?;

    services.users.remove_device(user_id, device_id).await;

    info!(
        ?user_id,
        ?device_id,
        "Session signed out via account management page"
    );

    let uid = html_escape(user_id.as_str());
    let did = html_escape(device_id.as_str());

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Session Signed Out</title>
	</head>
	<body>
		<main>
			<h1 class="ok">&check; Session Signed Out</h1>
			<p role="status">
				Session <code>{did}</code> for <strong>{uid}</strong> has been signed out.
			</p>
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
