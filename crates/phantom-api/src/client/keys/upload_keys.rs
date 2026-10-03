use axum::extract::State;
use phantom_core::{Err, Result, debug_warn};
use ruma::api::client::keys::upload_keys;

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/keys/upload`
///
/// Publish end-to-end encryption keys for the sender device.
///
/// - Adds one time keys, up to the configured per-device limit
/// - If there are no device keys yet: Adds device keys (TODO: merge with
///   existing keys?)
pub(crate) async fn upload_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<upload_keys::v3::Request>,
) -> Result<upload_keys::v3::Response> {
    let sender_user = body.sender_user();
    let sender_device = body.sender_device()?;

    if let Some(device_keys) = &body.device_keys {
        let keys = device_keys.deserialize()?;

        if keys.user_id != sender_user || keys.device_id != sender_device {
            return Err!(Request(Unknown(
                "Device keys do not belong to the authenticated device."
            )));
        }

        // Unchanged keys must not bump the device list version, or every
        // client re-downloads them.
        let unchanged = services
            .users
            .get_device_keys(sender_user, sender_device)
            .await
            .is_ok_and(|existing| existing.json().get() == device_keys.json().get());

        if !unchanged {
            services
                .users
                .add_device_keys(sender_user, sender_device, device_keys)
                .await;
        }
    }

    let limit = services.config.client.one_time_key_limit;
    let stored: usize = services
        .users
        .count_one_time_keys(sender_user, sender_device)
        .await
        .values()
        .map(|count| usize::try_from(u64::from(*count)).unwrap_or(usize::MAX))
        .sum();

    for (key_id, one_time_key) in body.one_time_keys.iter().take(limit.saturating_sub(stored)) {
        if let Err(e) = services
            .users
            .add_one_time_key(sender_user, sender_device, key_id, one_time_key)
            .await
        {
            debug_warn!(%sender_user, %sender_device, "Rejected one-time key {key_id}: {e}");
            return Err(e);
        }
    }

    Ok(upload_keys::v3::Response::new(
        services
            .users
            .count_one_time_keys(sender_user, sender_device)
            .await,
    ))
}
