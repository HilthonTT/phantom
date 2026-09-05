//! The state of one sliding sync conversation.
//!
//! A sliding sync request is a delta against the last one: a client that has
//! not changed its lists sends none, and a client that has changed one sends
//! only that one. So the server keeps the whole query and merges each request
//! into it — [`remember`] — and hands the merged query back for the endpoint
//! to answer against.
//!
//! It also keeps what it has already sent. A room the client has been given in
//! full does not need to be given again, and the position it was last sent at
//! is what decides whether it has changed since. That is [`known_rooms`], and
//! it is per list rather than per connection because the same room can appear
//! in two lists with different amounts of it sent.
//!
//! [`remember`]: super::Service::remember
//! [`known_rooms`]: Connection::known_rooms

use std::collections::BTreeMap;

use phantom_core::implement;
use ruma::{
    OwnedDeviceId, OwnedRoomId, OwnedUserId, UserId,
    api::client::sync::sync_events::v5::request::{Extensions, List, RoomSubscription},
};

use super::Service;

/// A user, one of their devices, and the connection id the client chose.
pub type ConnectionKey = (OwnedUserId, OwnedDeviceId, String);

/// What the server holds on behalf of one sliding sync conversation.
#[derive(Clone, Debug, Default)]
pub struct Connection {
    /// The lists the client has asked for, merged across every request it has
    /// made on this connection.
    pub lists: BTreeMap<String, List>,

    /// The rooms the client has asked to be told about regardless of any list.
    pub subscriptions: BTreeMap<OwnedRoomId, RoomSubscription>,

    /// Per list, the rooms already sent and the sync position they were sent
    /// at. A room whose position has not moved needs nothing sent for it.
    pub known_rooms: BTreeMap<String, BTreeMap<OwnedRoomId, u64>>,

    /// The extensions the client has turned on.
    pub extensions: Extensions,
}

/// Merges a request into the connection's state and returns the whole query.
///
/// A list or subscription the request does not mention is kept as it was; one
/// it does mention replaces what was there. That is the spec's rule, and the
/// reason a client can send an empty request and get a full answer.
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

    // Extensions are replaced wholesale rather than merged: unlike a list,
    // an extension the client stops sending is one it has turned off, and
    // there is no way to tell that apart from silence if they accumulate.
    connection.extensions = extensions;

    connection.clone()
}

/// The connection's state, without changing it.
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

/// Records which rooms were sent for one list, and at what position.
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

/// Forgets a conversation, at the client's request.
///
/// A client that has finished with a connection says so, and a server that
/// held the state until the process restarted would hold it for every client
/// that ever connected.
#[implement(Service)]
pub fn forget(&self, user_id: &UserId, device_id: &OwnedDeviceId, conn_id: Option<&str>) {
    let key = super::connection_key(&user_id.to_owned(), device_id, conn_id);

    self.connections.lock().expect("locked").remove(&key);
}

/// Forgets every conversation a device holds, which is what logging it out
/// does.
#[implement(Service)]
pub fn forget_device(&self, user_id: &UserId, device_id: &OwnedDeviceId) {
    self.connections
        .lock()
        .expect("locked")
        .retain(|(held_user, held_device, _), _| held_user != user_id || held_device != device_id);
}
