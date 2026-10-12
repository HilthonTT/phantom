use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::client::account::request_password_change_token_via_msisdn::{self, v3::Response};

use super::msisdn_token::{msisdn, require_sms, send_msisdn_token};
use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/account/password/msisdn/requestToken`
///
/// Texts a code that verifies a phone number before a password reset.
pub(crate) async fn request_password_change_token_via_msisdn_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<request_password_change_token_via_msisdn::v3::Request>,
) -> Result<Response> {
    require_sms(&services)?;

    let msisdn = msisdn(&body.country, &body.phone_number)?;

    // As for email, a reset for an unbound number reports not-found.
    if services
        .threepid
        .user_id_for_email(&msisdn)
        .await?
        .is_none()
    {
        return Err!(Request(ThreepidNotFound(
            "That phone number is not bound to an account"
        )));
    }

    let session = send_msisdn_token(
        &services,
        client,
        body.client_secret.as_str(),
        &msisdn,
        body.send_attempt.into(),
    )
    .await?;

    let mut response = Response::new(session.sid);
    response.submit_url = Some(session.submit_url);

    Ok(response)
}
