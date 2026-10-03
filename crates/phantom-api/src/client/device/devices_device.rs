use axum::extract::State;
use phantom_core::{Result, err};
use ruma::{
    MilliSecondsSinceUnixEpoch,
    api::client::device::{delete_device, get_device, update_device},
};

use crate::router::{ClientIp, Ruma, authenticate_uiaa as auth_uiaa};

/// # `GET /_matrix/client/r0/devices/{deviceId}`
///
/// Get metadata on a single device of the sender user.
pub(crate) async fn get_device_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_device::v3::Request>,
) -> Result<get_device::v3::Response> {
    let device = services
        .users
        .get_device_metadata(body.sender_user(), &body.body.device_id)
        .await
        .map_err(|_| err!(Request(NotFound("Device not found."))))?;

    Ok(get_device::v3::Response::new(device))
}

/// # `PUT /_matrix/client/r0/devices/{deviceId}`
///
/// Updates the metadata on a given device of the sender user.
#[tracing::instrument(skip_all, fields(%client), name = "update_device")]
pub(crate) async fn update_device_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<update_device::v3::Request>,
) -> Result<update_device::v3::Response> {
    let sender_user = body.sender_user();

    let mut device = services
        .users
        .get_device_metadata(sender_user, &body.device_id)
        .await
        .map_err(|_| err!(Request(NotFound("Device not found."))))?;

    device.display_name.clone_from(&body.display_name);
    device.last_seen_ip = Some(client.to_string());
    device.last_seen_ts = Some(MilliSecondsSinceUnixEpoch::now());

    services
        .users
        .update_device_metadata(sender_user, &body.device_id, &device)
        .await?;

    Ok(update_device::v3::Response::new())
}

/// # `DELETE /_matrix/client/r0/devices/{deviceId}`
///
/// Deletes the given device.
///
/// - Requires UIAA to verify user password
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip,
///   last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
pub(crate) async fn delete_device_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_device::v3::Request>,
) -> Result<delete_device::v3::Response> {
    let sender_user = &auth_uiaa(&services, &body).await?;

    services
        .users
        .remove_device(sender_user, &body.device_id)
        .await;

    Ok(delete_device::v3::Response::new())
}
