//! The phantom admin API, under `/_phantom/admin/v1`: JSON for the `phantom`
//! console. Every route takes an admin's access token, checked by
//! [`AdminAuth`].

use axum::{Json, Router, extract::State, response::IntoResponse, routing::get};
use phantom_core::Result;
use serde_json::json;

use crate::router::{AdminAuth, State as RouterState};

pub fn register(router: Router<RouterState>) -> Router<RouterState> {
    router.route("/_phantom/admin/v1/whoami", get(whoami))
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
