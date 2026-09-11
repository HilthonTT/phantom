use std::collections::BTreeMap;

use phantom_core::implement;
use ruma::{
    OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId,
    api::client::sync::sync_events::v5::request::{Extensions, List, RoomSubscription},
};

use super::Service;

pub type ConnectionKey = (OwnedUserId, OwnedDeviceId, String);

#[derive(Clone, Debug, Default)]
pub struct Connection {
    pub lists: BTreeMap<String, List>,

    pub subscriptions: BTreeMap<OwnedRoomId, RoomSubscription>,

    pub known_rooms: BTreeMap<String, BTreeMap<OwnedRoomId, u64>>,

    pub extensions: Extensions,
}

#[implement(Service)]
pub fn remember(
    &self,
    user_id: &UserId,
    device_id: &OwnedDeviceId,
    conn_id: Option<&str>,
    lists: BTreeMap<String, List>,
    subscriptions: BTreeMap<OwnedRoomId, RoomSubscription>,
    extensions: Extensions,
) -> Connection {
    let key = super::connection_key(&user_id.to_owned(), device_id, conn_id);
    let mut connections = self.connections.lock().expect("locked");
    let connection = connections.entry(key).or_default();

    connection.lists.extend(lists);
    connection.subscriptions.extend(subscriptions);

    connection.extensions = extensions;

    connection.clone()
}

#[implement(Service)]
#[must_use]
pub fn connection(
    &self,
    user_id: &UserId,
    device_id: &OwnedDeviceId,
    conn_id: Option<&str>,
) -> Option<Connection> {
    let key = super::connection_key(&user_id.to_owned(), device_id, conn_id);

    self.connections.lock().expect("locked").get(&key).cloned()
}

#[implement(Service)]
pub fn remember_rooms<I>(
    &self,
    user_id: &UserId,
    device_id: &OwnedDeviceId,
    conn_id: Option<&str>,
    list: &str,
    rooms: I,
) where
    I: IntoIterator<Item = (OwnedRoomId, u64)>,
{
    let key = super::connection_key(&user_id.to_owned(), device_id, conn_id);
    let mut connections = self.connections.lock().expect("locked");
    let connection = connections.entry(key).or_default();

    connection
        .known_rooms
        .entry(list.to_owned())
        .or_default()
        .extend(rooms);
}

#[implement(Service)]
pub fn forget(&self, user_id: &UserId, device_id: &OwnedDeviceId, conn_id: Option<&str>) {
    let key = super::connection_key(&user_id.to_owned(), device_id, conn_id);

    self.connections.lock().expect("locked").remove(&key);
}

#[implement(Service)]
pub fn forget_device(&self, user_id: &UserId, device_id: &OwnedDeviceId) {
    self.connections
        .lock()
        .expect("locked")
        .retain(|(held_user, held_device, _), _| held_user != user_id || held_device != device_id);
}
