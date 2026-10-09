use axum::{Json, extract::State, response::IntoResponse};
use futures::{StreamExt, future::join_all};
use phantom_core::{Result, stream::ReadyExt};
use ruma::{OwnedDeviceId, OwnedUserId, UserId};
use serde::Serialize;

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct User {
    user_id: OwnedUserId,
    display_name: Option<String>,

    admin: bool,
    deactivated: bool,

    /// The server's own user, which sends the admin room's notices.
    server_user: bool,

    devices: usize,
    rooms_joined: usize,

    /// When the account's most recently active device was last seen.
    last_seen_ms: Option<u64>,
}

/// # `GET /_phantom/admin/v1/users`
///
/// Every local account, deactivated ones included.
pub(super) async fn users(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let ids = local_users(&services).await;

    let users = join_all(ids.iter().map(|user_id| user(&services, user_id))).await;

    Ok(Json(users))
}

async fn user(services: &RouterState, user_id: &UserId) -> User {
    let devices: Vec<_> = services.users.all_devices_metadata(user_id).collect().await;

    let last_seen_ms = devices
        .iter()
        .filter_map(|device| device.last_seen_ts)
        .map(|ts| ts.get().into())
        .max();

    User {
        user_id: user_id.to_owned(),
        display_name: services.profile.displayname(user_id).await.ok(),
        admin: services.users.is_admin(user_id).await,
        deactivated: services
            .users
            .is_deactivated(user_id)
            .await
            .unwrap_or(false),
        server_user: user_id == services.server_state.server_user,
        devices: devices.len(),
        rooms_joined: services
            .rooms
            .state_cache
            .rooms_joined(user_id)
            .count()
            .await,
        last_seen_ms,
    }
}

#[derive(Serialize)]
pub(super) struct Device {
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    display_name: Option<String>,
    last_seen_ip: Option<String>,
    last_seen_ms: Option<u64>,
}

/// # `GET /_phantom/admin/v1/devices`
///
/// Every device of every local account.
pub(super) async fn devices(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let mut devices = Vec::new();

    for user_id in local_users(&services).await {
        let mut of_user: Vec<_> = services
            .users
            .all_devices_metadata(&user_id)
            .map(|device| Device {
                user_id: user_id.clone(),
                device_id: device.device_id,
                display_name: device.display_name,
                last_seen_ip: device.last_seen_ip,
                last_seen_ms: device.last_seen_ts.map(|ts| ts.get().into()),
            })
            .collect()
            .await;

        devices.append(&mut of_user);
    }

    Ok(Json(devices))
}

async fn local_users(services: &RouterState) -> Vec<OwnedUserId> {
    services
        .users
        .stream()
        .ready_filter(|user_id| services.server_state.user_is_local(user_id))
        .map(ToOwned::to_owned)
        .collect()
        .await
}
