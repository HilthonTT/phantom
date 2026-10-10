use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::IntoResponse,
};
use futures::StreamExt;
use phantom_core::{Err, Result, err};
use phantom_service::auth::registration_tokens::{TokenExpires, TokenInfo, ValidToken};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::router::{AdminAuth, State as RouterState};

/// What the config file's token shows as, matching the masked settings.
const MASKED: &str = "***********";

#[derive(Serialize)]
pub(super) struct Token {
    /// The token itself; a config-file token is masked, as in the settings.
    token: String,

    /// "config" for the config file's token, "database" for a created one.
    source: &'static str,

    uses: Option<u64>,
    max_uses: Option<u64>,
    expires_at_ms: Option<u64>,
}

/// # `GET /_phantom/admin/v1/registration_tokens`
///
/// The tokens registration currently accepts. Expired and used-up tokens are
/// dropped as the list is read, so they never appear.
pub(super) async fn tokens(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let tokens: Vec<Token> = services
        .registration_tokens
        .iterate_tokens()
        .await
        .map(token)
        .collect()
        .await;

    Ok(Json(tokens))
}

fn token(valid: ValidToken) -> Token {
    match valid.info {
        TokenInfo::Config => Token {
            token: MASKED.to_owned(),
            source: "config",
            uses: None,
            max_uses: None,
            expires_at_ms: None,
        },

        TokenInfo::Database(info) => Token {
            token: valid.token,
            source: "database",
            uses: Some(info.uses),
            max_uses: info.expires.max_uses,
            expires_at_ms: info
                .expires
                .max_age
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map(|since| since.as_millis().try_into().unwrap_or(u64::MAX)),
        },
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NewToken {
    /// The token to accept; a random one is made when absent.
    token: Option<String>,

    /// How many registrations it allows; unlimited when absent.
    uses_allowed: Option<u64>,

    /// How long it stays valid, in seconds; forever when absent.
    expires_in_secs: Option<u64>,
}

/// # `POST /_phantom/admin/v1/registration_tokens`
///
/// Creates a registration token, answering with it.
pub(super) async fn create(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: NewToken = super::body(&body)?;

    if body.token.as_deref().is_some_and(str::is_empty) {
        return Err!(Request(InvalidParam("The token cannot be empty.")));
    }

    let max_age = body
        .expires_in_secs
        .map(|secs| {
            SystemTime::now()
                .checked_add(Duration::from_secs(secs))
                .ok_or_else(|| err!(Request(InvalidParam("expires_in_secs is too far ahead."))))
        })
        .transpose()?;

    let (token, info) = services
        .registration_tokens
        .create_token(
            body.token.as_deref(),
            None,
            TokenExpires {
                max_uses: body.uses_allowed,
                max_age,
            },
        )
        .await?;

    Ok(Json(self::token(ValidToken {
        token,
        info: TokenInfo::Database(info),
    })))
}

/// # `DELETE /_phantom/admin/v1/registration_tokens/{token}`
///
/// Revokes a token. The config file's token can only be removed there.
pub(super) async fn revoke(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(token): Path<String>,
) -> Result<impl IntoResponse> {
    services.registration_tokens.revoke_token(&token).await?;

    Ok(Json(json!({})))
}
