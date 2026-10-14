//! The sync service against a real database: sliding sync connections that
//! outlive the in-memory cache, and watchers that are armed before a sync
//! samples its position.

use std::{sync::Arc, time::Duration};

use futures::FutureExt;
use phantom_core::time::now_millis;
use phantom_database::table;
use ruma::{
    DeviceId, UserId, api::client::sync::sync_events::v5::request::List, device_id, room_id, uint,
    user_id,
};
use tempfile::TempDir;

use super::Connection;
use crate::{
    Services,
    runtime::testing::{self, TestServices},
};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Every service built over a fresh database, without starting any worker.
fn services() -> TestServices {
    testing::build("")
}

/// Every service built over the database in `dir`.
fn services_at(dir: &TempDir) -> Arc<Services> {
    testing::build_at(dir, "")
}

fn connection(timeline_limit: u32) -> Connection {
    let mut list = List::default();
    list.ranges = vec![(uint!(0), uint!(20))];
    list.room_details.timeline_limit = timeline_limit.into();

    let mut connection = Connection::default();
    connection.lists.insert("all".to_owned(), list);
    connection.known_rooms.insert(
        "\0roomsince".to_owned(),
        [(room_id!("!room:phantom.test").to_owned(), 42)].into(),
    );
    connection.extensions.e2ee.enabled = Some(true);

    connection
}

fn json(connection: &Connection) -> String {
    serde_json::to_string(connection).expect("serializable")
}

async fn stored(
    test: &TestServices,
    user_id: &UserId,
    device_id: &DeviceId,
    conn_id: &str,
) -> bool {
    test.services.db[table::USERDEVICECONNID_SLIDINGSYNC]
        .contains(&(user_id, device_id, conn_id))
        .await
}

#[tokio::test]
async fn a_connection_survives_losing_the_cache() {
    let test = services();
    let sync = &test.services.sync;
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));

    sync.update(user, device, Some("main"), connection(10));
    sync.connections.lock().expect("locked").clear();

    let loaded = sync
        .connection(user, device, Some("main"))
        .await
        .expect("the connection was read back from the database");

    assert_eq!(json(&loaded), json(&connection(10)));
    assert!(
        sync.connection(user, device, Some("other")).await.is_none(),
        "an unknown conn_id stays unknown"
    );
}

#[tokio::test]
async fn a_connection_survives_a_restart() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));

    let services = services_at(&dir);
    services
        .sync
        .update(user, device, Some("main"), connection(10));
    drop(services);

    let services = services_at(&dir);
    let loaded = services
        .sync
        .connection(user, device, Some("main"))
        .await
        .expect("the connection was read back after the restart");

    assert_eq!(json(&loaded), json(&connection(10)));
}

#[tokio::test]
async fn an_unchanged_connection_is_not_rewritten() {
    let test = services();
    let sync = &test.services.sync;
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));
    let table = &test.services.db[table::USERDEVICECONNID_SLIDINGSYNC];

    sync.update(user, device, None, connection(10));
    assert!(stored(&test, user, device, "").await);

    // Remove the row behind the cache's back: an identical update must not
    // write it again, a changed one must.
    table.del((user, device, "")).expect("deleted");

    sync.update(user, device, None, connection(10));
    assert!(!stored(&test, user, device, "").await);

    sync.update(user, device, None, connection(20));
    assert!(stored(&test, user, device, "").await);
}

#[tokio::test]
async fn removing_a_device_forgets_only_its_connections() {
    let test = services();
    let sync = &test.services.sync;
    let user = user_id!("@alice:phantom.test");
    let (phone, laptop) = (device_id!("PHONE"), device_id!("LAPTOP"));

    sync.update(user, phone, Some("a"), connection(1));
    sync.update(user, phone, Some("b"), connection(2));
    sync.update(user, laptop, Some("a"), connection(3));

    sync.forget_device(user, phone).await;

    assert!(!stored(&test, user, phone, "a").await);
    assert!(!stored(&test, user, phone, "b").await);
    assert!(sync.connection(user, phone, Some("a")).await.is_none());
    assert!(stored(&test, user, laptop, "a").await);
    assert!(sync.connection(user, laptop, Some("a")).await.is_some());
}

#[tokio::test]
async fn an_idle_connection_expires() {
    let test = services();
    let sync = &test.services.sync;
    let user = user_id!("@alice:phantom.test");
    let (idle, busy) = (device_id!("IDLE"), device_id!("BUSY"));
    let table = &test.services.db[table::USERDEVICECONNID_SLIDINGSYNC];

    let last_used = now_millis().saturating_sub(30 * DAY_MS);
    let value = format!(
        r#"{{"last_used":{last_used},"connection":{}}}"#,
        json(&connection(1))
    );

    table
        .put_raw((user, idle, "a"), value.as_bytes())
        .expect("written");
    table
        .put_raw((user, idle, "b"), value.as_bytes())
        .expect("written");
    sync.update(user, busy, None, connection(1));

    // Read lazily, an expired connection is gone even before the sweep.
    assert!(sync.connection(user, idle, Some("a")).await.is_none());
    assert!(!stored(&test, user, idle, "a").await);

    assert_eq!(sync.expire_connections(now_millis()).await, 1);
    assert!(!stored(&test, user, idle, "b").await);
    assert!(stored(&test, user, busy, "").await);

    // The sweep also drops cached connections that went idle.
    assert_eq!(
        sync.expire_connections(now_millis().saturating_add(30 * DAY_MS))
            .await,
        1
    );
    assert!(sync.connections.lock().expect("locked").is_empty());
}

/// The race a long-poll used to lose: a write that lands after the sync
/// sampled its position, while it builds the response, but before it parks.
/// The watchers are armed before the sample now, so that write wakes them.
#[tokio::test]
async fn a_write_between_the_sample_and_the_wait_wakes_the_sync() {
    let test = services();
    let services = &test.services;
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));
    let room = room_id!("!room:phantom.test");

    services.db[table::USERROOMID_JOINED]
        .put_raw((user, room), [])
        .expect("joined");

    // What a sync does first: arm, then sample.
    let mut watch = services.sync.watch(user, device).await.expect("armed");
    let _next_batch = services.server_state.current_count();

    assert!(
        watch.as_mut().now_or_never().is_none(),
        "nothing happened yet, so the sync stays parked"
    );

    // A receipt in the joined room, written while the response is built.
    services.db[table::READRECEIPTID_READRECEIPT]
        .put_raw((room, 7_u64, user), b"{}")
        .expect("written");

    tokio::time::timeout(Duration::from_secs(5), watch)
        .await
        .expect("the write after the sample woke the parked sync");
}

#[tokio::test]
async fn a_typing_update_in_a_joined_room_wakes_the_sync() {
    let test = services();
    let services = &test.services;
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));
    let room = room_id!("!room:phantom.test");

    services.db[table::USERROOMID_JOINED]
        .put_raw((user, room), [])
        .expect("joined");

    let mut watch = services.sync.watch(user, device).await.expect("armed");
    let typing = &services.rooms.typing.typing_update_sender;

    typing
        .send(room_id!("!elsewhere:phantom.test").to_owned())
        .expect("a receiver is subscribed");
    tokio::task::yield_now().await;
    assert!(
        watch.as_mut().now_or_never().is_none(),
        "typing in another room leaves the sync parked"
    );

    typing
        .send(room.to_owned())
        .expect("a receiver is subscribed");

    tokio::time::timeout(Duration::from_secs(5), watch)
        .await
        .expect("typing in a joined room woke the sync");
}

#[tokio::test]
async fn deleting_a_device_drops_its_connections() {
    let test = services();
    let (user, device) = (user_id!("@alice:phantom.test"), device_id!("PHONE"));

    test.services.sync.update(user, device, None, connection(1));
    test.services.users.remove_device(user, device).await;

    assert!(!stored(&test, user, device, "").await);
}
