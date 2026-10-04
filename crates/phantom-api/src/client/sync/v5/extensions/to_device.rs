use futures::StreamExt;
use phantom_core::Result;
use ruma::api::client::sync::sync_events::v5::response;

use super::{Connection, SyncInfo};

#[tracing::instrument(name = "to_device", level = "trace", skip_all, ret)]
pub(super) async fn collect(
    SyncInfo {
        services,
        sender_user,
        sender_device,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
) -> Result<Option<response::ToDevice>> {
    let Some(sender_device) = sender_device else {
        return Ok(None);
    };

    // Per MSC3885, the to-device extension carries its own opaque `since` token
    // (independent of the global `pos`). matrix-rust-sdk persists it separately
    // from `pos`, so trust the request's claim when present and fall back to
    // `globalsince` only when the client did not provide one.
    let since = conn
        .extensions
        .to_device
        .since
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(conn.globalsince)
        .min(conn.next_batch);

    services
        .users
        .remove_to_device_events(sender_user, sender_device, since)
        .await;

    let events: Vec<_> = services
        .users
        .get_to_device_events(
            sender_user,
            sender_device,
            Some(since),
            Some(conn.next_batch),
        )
        .collect()
        .await;

    let to_device = (!events.is_empty()).then(|| {
        let mut to_device = response::ToDevice::default();
        to_device.next_batch = conn.next_batch.to_string();
        to_device.events = events;
        to_device
    });

    Ok(to_device)
}
