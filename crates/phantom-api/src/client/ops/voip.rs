use std::time::{Duration, SystemTime};

use axum::extract::State;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, KeyInit, Mac};
use phantom_core::{Err, Result};
use ruma::{SecondsSinceUnixEpoch, api::client::voip::get_turn_server_info};
use sha1::Sha1;

use crate::router::Ruma;

type HmacSha1 = Hmac<Sha1>;

/// # `GET /_matrix/client/r0/voip/turnServer`
///
/// Returns credentials for the configured TURN servers.
pub(crate) async fn turn_server_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_turn_server_info::v3::Request>,
) -> Result<get_turn_server_info::v3::Response> {
    let turn = &services.config.turn;

    // MSC4166: return M_NOT_FOUND 404 if no TURN URIs are specified in any way
    if turn.turn_uris.is_empty() {
        return Err!(Request(NotFound("Not Found")));
    }

    let user = body.sender_user();

    if !services.config.client.turn_allow_guests
        && body.appservice_info.is_none()
        && services.users.is_deactivated(user).await.unwrap_or(false)
    {
        return Err!(Request(Forbidden(
            "Guest users are not allowed to get TURN credentials"
        )));
    }

    let turn_secret = &services.server_state.turn_secret;
    let (username, password) = if turn_secret.is_empty() {
        (turn.turn_username.clone(), turn.turn_password.clone())
    } else {
        let expiry = SecondsSinceUnixEpoch::from_system_time(
            SystemTime::now()
                .checked_add(Duration::from_secs(turn.turn_ttl))
                .expect("TURN TTL should not get this high"),
        )
        .expect("time is valid");

        let username = format!("{}:{}", expiry.get(), user);
        let mut mac = HmacSha1::new_from_slice(turn_secret.as_bytes())
            .expect("HMAC can take key of any size");

        mac.update(username.as_bytes());
        let password = STANDARD.encode(mac.finalize().into_bytes());

        (username, password)
    };

    Ok(get_turn_server_info::v3::Response::new(
        username,
        password,
        turn.turn_uris.clone(),
        Duration::from_secs(turn.turn_ttl),
    ))
}
