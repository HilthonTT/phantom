//! Sliding sync (MSC4186) connections, kept in memory and persisted.
//!
//! A connection is what a client's `pos` resumes: its lists, subscriptions,
//! extension settings and how far each room was delivered. Losing it answers
//! the next request with `M_UNKNOWN_POS` and forces a full resync, so every
//! connection is written to `userdeviceconnid_slidingsync` and read back
//! lazily on the first request after a restart. The in-memory map is only a
//! cache in front of that table.

use std::collections::BTreeMap;

use futures::StreamExt;
use phantom_core::{debug, debug_warn, implement, stream::TryIgnore, time::now_millis};
use phantom_database::Interfix;
use ruma::{
    DeviceId, OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId,
    api::client::sync::sync_events::v5::request::{Extensions, List, RoomSubscription},
};
use serde::{Deserialize, Serialize};

use super::Service;

pub type ConnectionKey = (OwnedUserId, OwnedDeviceId, String);

/// How stale a persisted connection's last-use time may get before an
/// otherwise unchanged response rewrites it. Expiry is measured in days, so
/// an hour of drift never expires a connection still in use.
const TOUCH_INTERVAL_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Connection {
    #[serde(default)]
    pub lists: BTreeMap<String, List>,

    #[serde(default)]
    pub subscriptions: BTreeMap<OwnedRoomId, RoomSubscription>,

    #[serde(default)]
    pub known_rooms: BTreeMap<String, BTreeMap<OwnedRoomId, u64>>,

    #[serde(default)]
    pub extensions: Extensions,
}

/// A connection as the table holds it.
#[derive(Deserialize, Serialize)]
struct Stored {
    /// When a response last used the connection, in milliseconds since the
    /// epoch; the expiry sweep reads only this.
    last_used: u64,

    connection: Connection,
}

/// The last-use time alone, so the sweep skips decoding whole connections.
#[derive(Deserialize)]
struct LastUsed {
    last_used: u64,
}

/// A cached connection and what the table last received for it.
#[derive(Debug)]
pub(super) struct Cached {
    pub(super) connection: Connection,

    /// The connection's JSON as last written, compared against each update
    /// so an unchanged connection is not rewritten after every response.
    persisted: Vec<u8>,

    /// When the table's copy last had its last-use time written.
    persisted_at: u64,

    /// When a response last used the connection.
    pub(super) last_used: u64,
}

/// The connection resumed by `conn_id` on this device, loading it from the
/// database if this process has not served it yet.
///
/// A connection unused for longer than the configured expiry is deleted and
/// reads as absent even when the periodic sweep has not reached it yet.
#[implement(Service)]
pub async fn connection(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    conn_id: Option<&str>,
) -> Option<Connection> {
    let key = super::connection_key(user_id, device_id, conn_id);

    if let Some(cached) = self.connections.lock().expect("locked").get(&key) {
        return Some(cached.connection.clone());
    }

    let conn_id = conn_id.unwrap_or_default();
    let value = self
        .slidingsync
        .qry(&(user_id, device_id, conn_id))
        .await
        .ok()?;

    let stored: Stored = match serde_json::from_slice(&value) {
        Ok(stored) => stored,
        Err(e) => {
            debug_warn!(%user_id, %device_id, ?conn_id, "Dropping unreadable sliding sync connection: {e}");
            self.forget_stored(user_id, device_id, conn_id);
            return None;
        }
    };

    if self.expired(stored.last_used, now_millis()) {
        debug!(%user_id, %device_id, ?conn_id, "Sliding sync connection expired");
        self.forget_stored(user_id, device_id, conn_id);
        return None;
    }

    let persisted = serde_json::to_vec(&stored.connection).unwrap_or_default();
    let connection = stored.connection;

    // Another request may have loaded or replaced it meanwhile; the cached
    // entry is the newer one then.
    let mut connections = self.connections.lock().expect("locked");
    let cached = connections.entry(key).or_insert_with(|| Cached {
        connection,
        persisted,
        persisted_at: stored.last_used,
        last_used: stored.last_used,
    });

    Some(cached.connection.clone())
}

/// Replaces the connection with the state a response left it in.
///
/// The table is written only when the connection changed, or when its stored
/// last-use time is older than [`TOUCH_INTERVAL_MS`], so a long-polling
/// client that receives nothing new costs one serialization, not a write.
#[implement(Service)]
pub fn update(
    &self,
    user_id: &UserId,
    device_id: &DeviceId,
    conn_id: Option<&str>,
    connection: Connection,
) {
    let now = now_millis();
    let key = super::connection_key(user_id, device_id, conn_id);
    let serialized = match serde_json::to_vec(&connection) {
        Ok(serialized) => serialized,
        Err(e) => {
            debug_warn!(%user_id, %device_id, ?conn_id, "Cannot serialize sliding sync connection: {e}");
            Vec::new()
        }
    };

    let mut connections = self.connections.lock().expect("locked");
    let cached = connections.entry(key).or_insert_with(|| Cached {
        connection: Connection::default(),
        persisted: Vec::new(),
        persisted_at: 0,
        last_used: now,
    });

    let changed = cached.persisted != serialized;
    let stale = now.saturating_sub(cached.persisted_at) >= TOUCH_INTERVAL_MS;

    cached.connection = connection;
    cached.last_used = now;

    if serialized.is_empty() || !(changed || stale) {
        return;
    }

    // The connection is already serialized for the comparison above, so the
    // record is assembled around it rather than serialized a second time.
    let mut value = format!(r#"{{"last_used":{now},"connection":"#).into_bytes();
    value.extend_from_slice(&serialized);
    value.push(b'}');

    let conn_id = conn_id.unwrap_or_default();
    match self
        .slidingsync
        .put_raw((user_id, device_id, conn_id), &value)
    {
        Ok(()) => {
            cached.persisted = serialized;
            cached.persisted_at = now;
        }
        Err(e) => {
            debug_warn!(%user_id, %device_id, ?conn_id, "Cannot persist sliding sync connection: {e}");
        }
    }
}

/// Drops one connection, both cached and persisted.
#[implement(Service)]
pub fn forget(&self, user_id: &UserId, device_id: &DeviceId, conn_id: Option<&str>) {
    let key = super::connection_key(user_id, device_id, conn_id);

    self.connections.lock().expect("locked").remove(&key);
    self.forget_stored(user_id, device_id, conn_id.unwrap_or_default());
}

/// Drops every connection of a device, as when it is deleted or logs out.
#[implement(Service)]
pub async fn forget_device(&self, user_id: &UserId, device_id: &DeviceId) {
    self.connections
        .lock()
        .expect("locked")
        .retain(|(held_user, held_device, _), _| held_user != user_id || held_device != device_id);

    self.slidingsync
        .del_prefix(&(user_id, device_id, Interfix))
        .await;
}

/// Deletes every connection unused for longer than the configured expiry.
///
/// Returns how many persisted connections were removed.
#[implement(Service)]
pub async fn expire_connections(&self, now: u64) -> usize {
    self.connections
        .lock()
        .expect("locked")
        .retain(|_, cached| !self.expired(cached.last_used, now));

    let expired: Vec<Vec<u8>> = self
        .slidingsync
        .raw_stream()
        .ignore_err()
        .filter_map(async |(key, value)| {
            let expired = serde_json::from_slice::<LastUsed>(value)
                .is_ok_and(|stored| self.expired(stored.last_used, now));

            expired.then(|| key.to_vec())
        })
        .collect()
        .await;

    for key in &expired {
        self.slidingsync.remove(key).ok();
    }

    expired.len()
}

#[implement(Service)]
fn expired(&self, last_used: u64, now: u64) -> bool {
    let days = self
        .services
        .server
        .config
        .client
        .sliding_sync_connection_expiry_days;

    days != 0 && now.saturating_sub(last_used) > days.saturating_mul(24 * 60 * 60 * 1000)
}

#[implement(Service)]
fn forget_stored(&self, user_id: &UserId, device_id: &DeviceId, conn_id: &str) {
    self.slidingsync.del((user_id, device_id, conn_id)).ok();
}
