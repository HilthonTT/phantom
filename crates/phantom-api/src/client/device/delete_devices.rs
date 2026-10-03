use axum::extract::State;
use futures::StreamExt;
use phantom_core::{
    Result,
    stream::{IterStream, automatic_width},
};
use ruma::api::client::device::delete_devices::{self, v3::Response};

use crate::{router::Ruma, router::authenticate_uiaa as auth_uiaa};

/// # `POST /_matrix/client/v3/delete_devices`
///
/// Deletes the given list of devices.
///
/// - Requires UIAA to verify user password unless from an appservice with
///   MSC4190 enabled.
///
/// For each device:
/// - Invalidates access token
/// - Deletes device metadata (device id, device display name, last seen ip,
///   last seen ts)
/// - Forgets to-device events
/// - Triggers device list updates
pub(crate) async fn delete_devices_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_devices::v3::Request>,
) -> Result<Response> {
    let sender_user = &auth_uiaa(&services, &body).await?;

    body.devices
        .iter()
        .stream()
        .for_each_concurrent(automatic_width(), |device_id| {
            services.users.remove_device(sender_user, device_id)
        })
        .await;

    Ok(Response::new())
}
