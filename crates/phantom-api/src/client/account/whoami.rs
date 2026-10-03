use axum::extract::State;
use phantom_core::{Result, err};
use ruma::api::client::account::whoami;

use crate::router::Ruma;

/// # `GET _matrix/client/r0/account/whoami`
///
/// Get `user_id` of the sender user.
///
/// Note: Also works for Application Services
pub(crate) async fn whoami_route(
    State(services): State<crate::router::State>,
    body: Ruma<whoami::v3::Request>,
) -> Result<whoami::v3::Response> {
    let is_guest = body.appservice_info.is_none()
        && services
            .users
            .is_deactivated(body.sender_user())
            .await
            .map_err(|_| err!(Request(Forbidden("User does not exist."))))?;

    let mut response = whoami::v3::Response::new(body.sender_user().to_owned(), is_guest);
    response.device_id = body.sender_device.clone();

    Ok(response)
}
