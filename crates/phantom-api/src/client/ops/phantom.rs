use axum::{Json, extract::State, response::IntoResponse};
use futures::StreamExt;
use phantom_core::{Result, diagnostics::info};

/// # `GET /_phantom/server_version`
///
/// Phantom-specific API to get the server version, results akin to
/// `/_matrix/federation/v1/version`
pub(crate) async fn phantom_server_version() -> Result<impl IntoResponse> {
    Ok(Json(serde_json::json!({
        "name": info::name(),
        "version": info::version(),
    })))
}

/// # `GET /_phantom/local_user_count`
///
/// Phantom-specific API to return the amount of users registered on this
/// homeserver. Endpoint is disabled if federation is disabled for privacy. This
/// only includes active users (not deactivated, no guests, etc)
pub(crate) async fn phantom_local_user_count(
    State(services): State<crate::router::State>,
) -> Result<impl IntoResponse> {
    let user_count = services.users.list_local_users().count().await;

    Ok(Json(serde_json::json!({
        "count": user_count
    })))
}
