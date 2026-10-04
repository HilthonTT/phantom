mod connection;
mod extensions;
mod filter;
mod range;
mod rooms;
mod selector;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    time::Duration,
};

use axum::extract::State;
use futures::{FutureExt, future::join};
use phantom_core::{Err, Result, debug, debug_warn, result::LogErr, trace};
use phantom_service::Services;
use ruma::{
    DeviceId, OwnedDeviceId, OwnedRoomId, UserId,
    api::client::sync::sync_events::v5::{Request, Response, response},
    events::{
        GlobalAccountDataEventType, direct::DirectEventContent, room::member::MembershipState,
    },
};
use smallvec::SmallVec;
use tokio::time::{Instant, timeout_at};

use self::{
    connection::Connection,
    extensions::{apply_profiles, apply_ranges, handle as handle_extensions},
    range::collect as collect_ranges,
};
use crate::router::Ruma;

#[derive(Copy, Clone)]
struct SyncInfo<'a> {
    services: &'a Services,
    sender_user: &'a UserId,
    sender_device: Option<&'a DeviceId>,
    previous_connection_pos: Option<u64>,

    /// The rooms the user calls direct, read once for the whole pass.
    ///
    /// Both the list filters and every room payload answer `is_dm` from this.
    direct_rooms: &'a BTreeSet<OwnedRoomId>,
}

#[derive(Clone, Debug)]
struct WindowRoom {
    room_id: OwnedRoomId,
    membership: Option<MembershipState>,
    lists: ListIds,
    event_count: u64,
    payload_count: u64,
}

impl WindowRoom {
    #[inline]
    fn payload_is_fresh(&self, roomsince: u64) -> bool {
        roomsince == 0 || self.payload_count > roomsince
    }
}

type ListId = String;
type Window = BTreeMap<OwnedRoomId, WindowRoom>;
type ResponseLists = BTreeMap<ListId, response::List>;
type ListIds = SmallVec<[ListId; 1]>;

/// `POST /_matrix/client/unstable/org.matrix.simplified_msc3575/sync`
/// ([MSC4186])
///
/// A simplified version of sliding sync ([MSC3575]).
///
/// Get all new events in a sliding window of rooms since the last sync or a
/// given point in time.
///
/// [MSC3575]: https://github.com/matrix-org/matrix-spec-proposals/pull/3575
/// [MSC4186]: https://github.com/matrix-org/matrix-spec-proposals/pull/4186
#[tracing::instrument(
    name = "sync",
    level = "debug",
    skip_all,
    fields(
        user_id = %body.sender_user().localpart(),
        device_id = %body.sender_device.as_deref().map_or("<no device>", |x| x.as_str()),
        conn_id = ?body.body.conn_id.clone().unwrap_or_default(),
        since = ?body.body.pos.clone().unwrap_or_default(),
    )
)]
pub(crate) async fn sync_events_v5_route(
    State(services): State<crate::router::State>,
    body: Ruma<Request>,
) -> Result<Response> {
    let services = &*services;
    let sender_user = body.sender_user();
    let sender_device = body.sender_device.as_deref();
    let request = &body.body;
    let since = request
        .pos
        .as_ref()
        .and_then(|string| string.parse().ok())
        .unwrap_or(0);

    let timeout = request
        .timeout
        .as_ref()
        .map(Duration::as_millis)
        .and_then(|timeout| u64::try_from(timeout).ok())
        .map(|timeout: u64| timeout.min(services.config.client.client_sync_timeout_max))
        .unwrap_or(0);

    // phantom's connection store is keyed by a device; a deviceless sender
    // (an appservice without device masquerading) shares one empty device.
    let device_key: OwnedDeviceId = sender_device.map_or_else(|| "".into(), ToOwned::to_owned);
    let conn_id = request.conn_id.as_deref();

    if services.config.client.allow_local_presence {
        services
            .presence
            .ping_presence(sender_user, &request.set_presence)
            .await
            .log_err()
            .ok();
    }

    let stored = services.sync.connection(sender_user, &device_key, conn_id);

    if since != 0 && stored.is_none() {
        return Err!(Request(UnknownPos(warn!(
            "Connection lost; restarting sync stream."
        ))));
    }

    let mut conn = match stored {
        Some(stored) if since != 0 => Connection::load(stored),
        _ => {
            services.sync.forget(sender_user, &device_key, conn_id);

            debug_warn!(?conn_id, "Client cleared cache and reloaded.");
            Connection::default()
        }
    };

    // Update parameters regardless of replay or advance
    conn.next_batch = services.server_state.current_count();
    conn.globalsince = since.min(conn.next_batch);
    let config_changed = conn.update_cache(request);
    let caught_up = conn.globalsince == conn.next_batch;

    // A whole profile owed to a caught-up connection needs a pass like a new list.
    let needs_pass = config_changed || conn.own_profile_owed();

    if config_change_needs_position(needs_pass, caught_up, since) {
        conn.next_batch = services.server_state.next_count()?;
    }

    // phantom keeps no record of the last issued position, so every resumed
    // position rewinds the rooms that progressed past it.
    conn.update_rooms_prologue((since != 0).then_some(since));

    let mut response = Response::new(String::new());
    response.txn_id.clone_from(&request.txn_id);

    let stop_at = Instant::now()
        .checked_add(Duration::from_millis(timeout))
        .expect("configuration must limit maximum timeout");

    loop {
        debug_assert!(
            conn.globalsince <= conn.next_batch,
            "since should not be greater than next_batch."
        );

        let window;
        let watchers = services.sync.watch(sender_user, &device_key);

        let direct_rooms = direct_rooms(services, sender_user).await;
        let sync_info = SyncInfo {
            services,
            sender_user,
            sender_device,
            previous_connection_pos: since.ne(&0).then_some(since),
            direct_rooms: &direct_rooms,
        };

        (window, response.lists) = selector::selector(&mut conn, sync_info).boxed().await;

        if conn.globalsince < conn.next_batch {
            let ranges = collect_ranges(sync_info, &conn, &window);
            let extensions = handle_extensions(sync_info, &conn, &window);
            let (mut ranges, extensions) = join(ranges, extensions).boxed().await;

            let mut extensions = extensions?;

            apply_profiles(sync_info, &conn, &window, &ranges, &mut extensions).await?;
            apply_ranges(&conn, &window, &mut ranges, &mut extensions);
            conn.update_rooms_epilogue(ranges.room_updates());
            response.rooms = ranges.into_payloads();
            response.extensions = extensions.into_response(&response.rooms);

            if !is_empty_response(&response) {
                response.pos = conn.next_batch.to_string();
                trace!(conn.globalsince, conn.next_batch, "response {response:?}");
                conn.store(services, sender_user, &device_key, conn_id);
                return Ok(response);
            }
        }

        let waiter = async || {
            tokio::select! {
                () = services.server.until_shutdown() => true,
                watch = timeout_at(stop_at, watchers) => watch.is_err(),
            }
        };

        if timeout == 0 || services.server.is_stopping() || waiter().boxed().await {
            response.pos = conn.next_batch.to_string();
            trace!(
                conn.globalsince,
                conn.next_batch, "empty response {response:?}"
            );
            conn.store(services, sender_user, &device_key, conn_id);
            return Ok(response);
        }

        debug!(
            ?timeout,
            last_since = conn.globalsince,
            last_batch = conn.next_batch,
            "notified by watcher"
        );

        conn.globalsince = conn.next_batch;
        conn.next_batch = services.server_state.current_count();
    }
}

/// The rooms named in the user's `m.direct` account data.
async fn direct_rooms(services: &Services, sender_user: &UserId) -> BTreeSet<OwnedRoomId> {
    services
        .account_data
        .get_global::<DirectEventContent>(sender_user, GlobalAccountDataEventType::Direct)
        .await
        .map(|content| content.0.into_values().flatten().collect())
        .unwrap_or_default()
}

fn config_change_needs_position(config_changed: bool, caught_up: bool, since: u64) -> bool {
    config_changed && caught_up && since != 0
}

fn is_empty_response(response: &Response) -> bool {
    response.extensions.is_empty() && response.rooms.is_empty()
}

#[cfg(test)]
mod tests {
    use super::config_change_needs_position;

    #[test]
    fn caught_up_config_change_needs_position() {
        assert!(config_change_needs_position(true, true, 1));
        assert!(!config_change_needs_position(false, true, 1));
        assert!(!config_change_needs_position(true, false, 1));
        assert!(!config_change_needs_position(true, true, 0));
    }
}
