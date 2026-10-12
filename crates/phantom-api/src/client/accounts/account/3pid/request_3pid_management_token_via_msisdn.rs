use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::client::account::request_3pid_management_token_via_msisdn::{self, v3::Response};

use super::msisdn_token::{msisdn, require_sms, send_msisdn_token};
use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/account/3pid/msisdn/requestToken`
///
/// Texts a code that verifies a phone number before it is added to the
/// account.
pub(crate) async fn request_3pid_management_token_via_msisdn_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<request_3pid_management_token_via_msisdn::v3::Request>,
) -> Result<Response> {
    require_sms(&services)?;

    let msisdn = msisdn(&body.country, &body.phone_number)?;

    if services.threepid.address_in_use(&msisdn).await {
        return Err!(Request(ThreepidInUse(
            "That phone number is already in use"
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
