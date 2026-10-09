use std::time::UNIX_EPOCH;

use axum::{Json, extract::State, response::IntoResponse};
use futures::StreamExt;
use phantom_core::Result;
use phantom_service::auth::registration_tokens::{TokenInfo, ValidToken};
use serde::Serialize;

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
