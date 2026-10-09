//! The phantom admin API, under `/_phantom/admin/v1`: JSON for the `phantom`
//! console. Every route takes an admin's access token, checked by
//! [`AdminAuth`].

mod appservices;
mod rooms;
mod settings;
mod stats;
mod tokens;
mod users;

use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use phantom_core::Result;
use serde_json::json;

use crate::router::{AdminAuth, State as RouterState};

pub fn register(router: Router<RouterState>) -> Router<RouterState> {
    router
        .route("/_phantom/admin/v1/whoami", get(whoami))
        .route("/_phantom/admin/v1/stats", get(stats::stats))
        .route("/_phantom/admin/v1/users", get(users::users))
        .route("/_phantom/admin/v1/devices", get(users::devices))
        .route(
            "/_phantom/admin/v1/registration_tokens",
            get(tokens::tokens),
        )
        .route("/_phantom/admin/v1/rooms", get(rooms::rooms))
        .route(
            "/_phantom/admin/v1/appservices",
            get(appservices::appservices),
        )
        .route("/_phantom/admin/v1/settings", get(settings::settings))
}

/// # `GET /_phantom/admin/v1/whoami`
///
/// The admin the token belongs to, and the server they administer.
async fn whoami(
    State(services): State<RouterState>,
    admin: AdminAuth,
) -> Result<impl IntoResponse> {
    Ok(Json(json!({
        "user_id": admin.user_id,
        "device_id": admin.device_id,
        "server_name": services.server.name,
    })))
}
