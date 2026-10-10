//! The phantom admin API, under `/_phantom/admin/v1`: JSON for the `phantom`
//! console. Every route takes an admin's access token, checked by
//! [`AdminAuth`].

mod appservices;
mod insight;
mod ops;
mod rooms;
mod settings;
mod stats;
mod tokens;
mod users;

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use phantom_core::{Result, err};
use serde::de::DeserializeOwned;
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
        .route("/_phantom/admin/v1/tasks", get(ops::tasks))
        .route(
            "/_phantom/admin/v1/users/{user_id}/deactivate",
            post(users::deactivate),
        )
        .route(
            "/_phantom/admin/v1/users/{user_id}/password",
            put(users::password),
        )
        .route(
            "/_phantom/admin/v1/users/{user_id}/admin",
            put(users::grant_admin).delete(users::revoke_admin),
        )
        .route(
            "/_phantom/admin/v1/devices/{user_id}/{device_id}",
            delete(users::delete_device),
        )
        .route(
            "/_phantom/admin/v1/registration_tokens",
            post(tokens::create),
        )
        .route(
            "/_phantom/admin/v1/registration_tokens/{token}",
            delete(tokens::revoke),
        )
        .route(
            "/_phantom/admin/v1/rooms/{room_id}/ban",
            put(rooms::ban).delete(rooms::unban),
        )
        .route(
            "/_phantom/admin/v1/rooms/{room_id}/shutdown",
            post(rooms::shutdown),
        )
        .route("/_phantom/admin/v1/rooms/{room_id}", delete(rooms::delete))
        .route("/_phantom/admin/v1/config/reload", post(ops::reload))
        .route("/_phantom/admin/v1/backup", post(ops::backup))
        .route("/_phantom/admin/v1/services", get(insight::services))
        .route("/_phantom/admin/v1/federation", get(insight::federation))
        .route(
            "/_phantom/admin/v1/federation/{server_name}/media",
            delete(insight::purge_remote_media),
        )
        .route("/_phantom/admin/v1/media", get(insight::media))
        .route(
            "/_phantom/admin/v1/media/{server_name}/{media_id}",
            delete(insight::delete_media),
        )
        .route("/_phantom/admin/v1/logs", get(insight::logs))
        .route("/_phantom/admin/v1/reports", get(insight::reports))
        .route(
            "/_phantom/admin/v1/reports/{id}",
            delete(insight::dismiss_report),
        )
}

/// Reads an action's JSON body; an empty body reads as `{}`, so optional
/// fields take their defaults and a required one is reported missing.
fn body<T: DeserializeOwned>(bytes: &Bytes) -> Result<T> {
    let bytes: &[u8] = if bytes.is_empty() { b"{}" } else { bytes };

    serde_json::from_slice(bytes).map_err(|e| err!(Request(BadJson("{e}"))))
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
