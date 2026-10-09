use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::IntoResponse,
};
use futures::{StreamExt, future::join_all};
use phantom_core::{Err, Result, stream::ReadyExt};
use ruma::{OwnedDeviceId, OwnedUserId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Deactivate {
    /// Also wipe the account's data and leave its profile empty for good.
    erase: bool,
}

/// # `POST /_phantom/admin/v1/users/{user_id}/deactivate`
///
/// Deactivates a local account: its devices are signed out, its password is
/// cleared, it leaves every room, and it gives up any admin rights.
pub(super) async fn deactivate(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path(user_id): Path<OwnedUserId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Deactivate = super::body(&body)?;
    local_account(&services, &user_id).await?;

    if user_id == admin.user_id {
        return Err!(Request(Forbidden(
            "You cannot deactivate your own account here."
        )));
    }

    services
        .deactivate
        .full_deactivate(&user_id, body.erase)
        .await?;

    Ok(Json(json!({})))
}

#[derive(Deserialize)]
pub(super) struct Password {
    password: String,

    /// Sign the account's devices out, so the old password's sessions end.
    #[serde(default = "true_fn")]
    logout_devices: bool,
}

fn true_fn() -> bool {
    true
}

/// # `PUT /_phantom/admin/v1/users/{user_id}/password`
///
/// Sets a local account's password, signing its devices out unless asked
/// not to. An admin resetting their own keeps the device they asked from.
pub(super) async fn password(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path(user_id): Path<OwnedUserId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Password = super::body(&body)?;
    local_account(&services, &user_id).await?;

    if body.password.is_empty() {
        return Err!(Request(InvalidParam("The password cannot be empty.")));
    }

    services
        .users
        .set_password(&user_id, Some(&body.password))?;

    if body.logout_devices {
        let devices: Vec<OwnedDeviceId> = services
            .users
            .all_device_ids(&user_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        for device_id in devices {
            if user_id != admin.user_id || device_id != admin.device_id {
                services.users.remove_device(&user_id, &device_id).await;
            }
        }
    }

    Ok(Json(json!({})))
}

/// # `PUT /_phantom/admin/v1/users/{user_id}/admin`
///
/// Makes a user an admin by joining them to the admin room; a remote user is
/// invited instead.
pub(super) async fn grant_admin(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(user_id): Path<OwnedUserId>,
) -> Result<impl IntoResponse> {
    if services.server_state.user_is_local(&user_id) {
        local_account(&services, &user_id).await?;
    }

    services.admin.make_user_admin(&user_id).await?;

    Ok(Json(json!({})))
}

/// # `DELETE /_phantom/admin/v1/users/{user_id}/admin`
///
/// Takes a user's admin rights away. An admin cannot revoke their own, so the
/// server is never left without one by accident.
pub(super) async fn revoke_admin(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path(user_id): Path<OwnedUserId>,
) -> Result<impl IntoResponse> {
    if user_id == admin.user_id {
        return Err!(Request(Forbidden(
            "You cannot revoke your own admin; ask another admin."
        )));
    }

    services.admin.revoke_admin(&user_id).await?;

    Ok(Json(json!({})))
}

/// # `DELETE /_phantom/admin/v1/devices/{user_id}/{device_id}`
///
/// Signs a device out, removing its tokens and keys. The device the request
/// came from is refused; sign out with the console's own logout instead.
pub(super) async fn delete_device(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path((user_id, device_id)): Path<(OwnedUserId, OwnedDeviceId)>,
) -> Result<impl IntoResponse> {
    if user_id == admin.user_id && device_id == admin.device_id {
        return Err!(Request(Forbidden(
            "This is the device you are signed in with."
        )));
    }

    if !services.users.device_exists(&user_id, &device_id).await {
        return Err!(Request(NotFound("{user_id} has no device {device_id}.")));
    }

    services.users.remove_device(&user_id, &device_id).await;

    Ok(Json(json!({})))
}

/// Refuses anything but an existing local account other than the server
/// user, which the admin actions must never touch.
async fn local_account(services: &RouterState, user_id: &UserId) -> Result {
    if !services.server_state.user_is_local(user_id) {
        return Err!(Request(InvalidParam("{user_id} is not a local account.")));
    }
    if *user_id == services.server_state.server_user {
        return Err!(Request(Forbidden("The server user cannot be changed.")));
    }
    if !services.users.exists(user_id).await {
        return Err!(Request(NotFound("{user_id} does not exist.")));
    }

    Ok(())
}
