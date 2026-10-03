use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::{
    MilliSecondsSinceUnixEpoch,
    api::client::account::add_3pid::{self, v3::Response},
};

use crate::{
    router::authenticate_uiaa as auth_uiaa,
    router::{ClientIp, Ruma},
};

/// # `POST /_matrix/client/v3/account/3pid/add`
///
/// Bind a verified email address to this account.
///
/// - Requires UIAA to confirm account ownership
/// - Consumes a previously validated email verification session
#[tracing::instrument(skip_all, fields(%client), name = "add_3pid")]
pub(crate) async fn add_3pid_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<add_3pid::v3::Request>,
) -> Result<Response> {
    if !services.sendmail.is_enabled() {
        return Err!(Request(ThreepidDenied(
            "Email verification is not configured"
        )));
    }

    let sender_user = &auth_uiaa(&services, &body).await?;

    let association = services
        .threepid
        .redeem_validated(body.sid.as_str(), body.client_secret.as_str())
        .await?;

    if services
        .threepid
        .user_id_for_email(&association.address)
        .await?
        .is_some_and(|bound| bound != *sender_user)
    {
        return Err!(Request(ThreepidInUse(
            "That email address is already in use"
        )));
    }

    let now = MilliSecondsSinceUnixEpoch::now();

    services
        .threepid
        .put_binding(
            sender_user,
            &association.address,
            association.medium,
            now,
            now,
        )
        .await?;

    Ok(Response::new())
}
