use std::{cmp, fmt::Write};

use futures::StreamExt;
use phantom_core::{Result, html::escape as html_escape};
use phantom_service::Services;
use ruma::UserId;

use super::{ACCOUNT_HEAD, ts_cell, url_encode};

pub(super) async fn sessions_list_html(services: &Services, user_id: &UserId) -> Result<String> {
    let mut devices: Vec<_> = services.users.all_devices_metadata(user_id).collect().await;

    devices.sort_by_key(|device| cmp::Reverse(device.last_seen_ts));

    let rows = devices.iter().fold(String::new(), |mut rows, device| {
        let name = html_escape(device.display_name.as_deref().unwrap_or("Unknown device"));
        let id = html_escape(device.device_id.as_str());
        let id_enc = url_encode(device.device_id.as_str());
        let ip = html_escape(device.last_seen_ip.as_deref().unwrap_or("—"));
        let ts = ts_cell(device.last_seen_ts);

        write!(
            rows,
            r#"
			<tr>
				<td>{name}</td>
				<td><code>{id}</code></td>
				<td>{ip}</td>
				<td>{ts}</td>
				<td class="center">
					<a href="/_phantom/oidc/account?action=org.matrix.session_view&device_id={id_enc}">
						View
					</a>
					<span class="sep"> | </span>
					<a
						href="/_phantom/oidc/account?action=org.matrix.session_end&device_id={id_enc}"
						class="err"
					>
						Sign out
					</a>
				</td>
			</tr>"#
        )
        .ok();

        rows
    });

    let uid = html_escape(user_id.as_str());
    let count = devices.len();

    // format! substitutes in one pass, so user-controlled values (display
    // names, device IDs) can't be mistaken for a placeholder.
    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
	<head>
		{ACCOUNT_HEAD}
		<title>Active Sessions</title>
	</head>
	<body class="wide">
		<h1>Active Sessions</h1>
		<p>
			Signed in as <strong>{uid}</strong>. {count} active session(s).
		</p>
		<table>
			<tr>
				<th>Name</th>
				<th>Device ID</th>
				<th>Last seen IP</th>
				<th>Last seen</th>
				<th class="center">Actions</th>
			</tr>
			{rows}
		</table>
		<div class="nav">
			<a href="/_phantom/oidc/account?action=org.matrix.profile">View Profile</a>
		</div>
	</body>
</html>"#
    ))
}
