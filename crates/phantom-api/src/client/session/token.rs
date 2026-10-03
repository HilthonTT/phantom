use std::time::Duration;

use axum::extract::State;
use phantom_core::{Err, Result, rand};
use phantom_service::Services;
use ruma::{
    OwnedUserId,
    api::client::session::{
        get_login_token,
        login::v3::{Request, Token},
    },
};

use super::super::TOKEN_LENGTH;
use crate::router::{ClientIp, Ruma, authenticate_uiaa as auth_uiaa};

pub(super) async fn handle_login(
    services: &Services,
    _body: &Ruma<Request>,
    info: &Token,
) -> Result<OwnedUserId> {
    let token = &info.token;

    if !services.config.client.login_via_token {
        return Err!(Request(Unknown("Token login is not enabled.")));
    }

    services.users.find_from_login_token(token).await
}

/// # `POST /_matrix/client/v1/login/get_token`
///
/// Allows a logged-in user to get a short-lived token which can be used
/// to log in with the m.login.token flow.
///
/// <https://spec.matrix.org/v1.13/client-server-api/#post_matrixclientv1loginget_token>
#[tracing::instrument(skip_all, fields(%client), name = "login_token")]
pub(crate) async fn login_token_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_login_token::v1::Request>,
) -> Result<get_login_token::v1::Response> {
    if !services.config.client.login_via_existing_session || !services.config.client.login_via_token
    {
        return Err!(Request(Forbidden(
            "Login via an existing session is not enabled"
        )));
    }

    let sender_user = auth_uiaa(&services, &body).await?;
    if !services.users.is_active_local(&sender_user).await {
        return Err!(Request(UserDeactivated("This user has been deactivated.")));
    }

    let login_token = rand::string(TOKEN_LENGTH);
    let expires_in = services
        .users
        .create_login_token(&sender_user, &login_token);

    Ok(get_login_token::v1::Response::new(
        Duration::from_millis(expires_in),
        login_token,
    ))
}
