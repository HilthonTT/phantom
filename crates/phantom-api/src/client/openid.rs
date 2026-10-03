use std::time::Duration;

use axum::extract::State;
use phantom_core::{Err, Result, rand};
use ruma::{api::client::account::request_openid_token, authentication::TokenType};

use super::TOKEN_LENGTH;
use crate::router::Ruma;

/// # `POST /_matrix/client/v3/user/{userId}/openid/request_token`
///
/// Request an OpenID token to verify identity with third-party services.
///
/// - The token generated is only valid for the OpenID API
pub(crate) async fn create_openid_token_route(
    State(services): State<crate::router::State>,
    body: Ruma<request_openid_token::v3::Request>,
) -> Result<request_openid_token::v3::Response> {
    let sender_user = body.sender_user();

    if sender_user != body.user_id {
        return Err!(Request(InvalidParam(
            "Not allowed to request OpenID tokens on behalf of other users"
        )));
    }

    let access_token = rand::string(TOKEN_LENGTH);
    let expires_in = services
        .users
        .create_openid_token(&body.user_id, &access_token)?;

    Ok(request_openid_token::v3::Response::new(
        access_token,
        TokenType::Bearer,
        services.server_state.server_name().to_owned(),
        Duration::from_secs(expires_in),
    ))
}
