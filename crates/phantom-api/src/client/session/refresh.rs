use axum::extract::State;
use phantom_core::{Err, Error, Result, debug_info, time::timepoint_has_passed};
use phantom_service::accounts::users::{RefreshToken, generate_refresh_token};
use ruma::api::{
    client::session::refresh_token::v3::{Request, Response},
    error::{ErrorKind, UnknownTokenErrorData},
};

use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/refresh`
///
/// Refresh an access token.
///
/// <https://spec.matrix.org/v1.15/client-server-api/#post_matrixclientv3refresh>
#[tracing::instrument(skip_all, fields(%client), name = "refresh_token")]
pub(crate) async fn refresh_token_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<Request>,
) -> Result<Response> {
    let refresh_token_claim = body.body.refresh_token;

    if !refresh_token_claim.starts_with("refresh_") {
        return Err!(Request(Forbidden("Refresh token is malformed.")));
    }

    match services
        .users
        .classify_refresh_token(&refresh_token_claim)
        .await
    {
        RefreshToken::Current {
            user_id,
            device_id,
            expires_at,
        } => {
            if expires_at.is_some_and(timepoint_has_passed) {
                let hard = services.server.config.auth.refresh_token_hard_logout;
                if hard {
                    services.users.remove_device(&user_id, &device_id).await;
                } else {
                    services
                        .users
                        .remove_refresh_token(&user_id, &device_id)
                        .await
                        .ok();
                }

                return Err(unknown_token(!hard, "Refresh token has expired."));
            }

            let refresh_token = Some(generate_refresh_token());
            let (access_token, expires_in_ms) = services.users.generate_access_token(true);

            services
                .users
                .set_access_token(
                    &user_id,
                    &device_id,
                    &access_token,
                    expires_in_ms,
                    refresh_token.as_deref(),
                )
                .await?;

            debug_info!(
                ?user_id,
                ?device_id,
                ?expires_in_ms,
                "refreshed their access_token"
            );

            let mut response = Response::new(access_token);
            response.refresh_token = refresh_token;
            response.expires_in_ms = expires_in_ms;

            Ok(response)
        }

        RefreshToken::Replayed {
            user_id,
            device_id,
            current,
            grace,
        } if grace => {
            // Benign double-submit: re-issue an access token for the unchanged
            // refresh token rather than rotating it.
            let (access_token, expires_in_ms) = services.users.generate_access_token(true);

            services
                .users
                .set_access_token(&user_id, &device_id, &access_token, expires_in_ms, None)
                .await?;

            let mut response = Response::new(access_token);
            response.refresh_token = Some(current);
            response.expires_in_ms = expires_in_ms;

            Ok(response)
        }

        RefreshToken::Replayed {
            user_id, device_id, ..
        } => {
            let revoke = services.server.config.auth.refresh_token_reuse_revoke;
            debug_info!(
                ?user_id,
                ?device_id,
                revoke,
                "refresh token reused after rotation"
            );

            if revoke {
                services.users.remove_device(&user_id, &device_id).await;
            }

            Err(unknown_token(
                !revoke,
                "Refresh token has already been used.",
            ))
        }

        RefreshToken::Unknown => Err!(Request(Forbidden("Refresh token is unrecognized."))),
    }
}

fn unknown_token(soft_logout: bool, message: &'static str) -> Error {
    let mut data = UnknownTokenErrorData::new();
    data.soft_logout = soft_logout;

    Error::BadRequest(ErrorKind::UnknownToken(data), message)
}
