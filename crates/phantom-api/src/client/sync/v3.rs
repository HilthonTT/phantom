mod joined;
mod left;
mod state;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    pin::pin,
    time::Duration,
};

use axum::extract::State;
use futures::{
    FutureExt, StreamExt, TryFutureExt,
    future::{OptionFuture, join3, join4, join5},
};
use phantom_core::{
    Result, error, extract_variant,
    future::TryExt,
    result::{FlatOk, LogErr},
    stream::{BroadbandExt, IterStream, ReadyExt, Tools},
    trace,
};
use phantom_service::{Services, accounts::account_data::AnyRawAccountDataEvent};
use ruma::{
    DeviceId, OwnedUserId, UserId,
    api::client::{
        filter::FilterDefinition,
        sync::sync_events::{
            self, DeviceLists,
            v3::{
                Filter, GlobalAccountData, InviteState, InvitedRoom, KnockState, KnockedRoom,
                Presence, Rooms, ToDevice,
            },
        },
    },
    events::presence::{PresenceEvent, PresenceEventContent},
    serde::Raw,
};
use tokio::time;

use super::share_encrypted_room;
use crate::router::Ruma;

use self::{joined::load_joined_room, left::handle_left_room};

type PresenceUpdates = HashMap<OwnedUserId, PresenceEventContent>;

/// # `GET /_matrix/client/r0/sync`
///
/// Synchronize the client's state with the latest state on the server.
///
/// - This endpoint takes a `since` parameter which should be the `next_batch`
///   value from a previous request for incremental syncs.
///
/// Calling this endpoint without a `since` parameter returns:
/// - Some of the most recent events of each timeline
/// - Notification counts for each room
/// - Joined and invited member counts, heroes
/// - All state events
///
/// Calling this endpoint with a `since` parameter from a previous `next_batch`
/// returns: For joined rooms:
/// - Some of the most recent events of each timeline that happened after since
/// - If user joined the room after since: All state events (unless lazy loading
///   is activated) and all device list updates in that room
/// - If the user was already in the room: A list of all events that are in the
///   state now, but were not in the state at `since`
/// - If the state we send contains a member event: Joined and invited member
///   counts, heroes
/// - Device list updates that happened after `since`
/// - If there are events in the timeline we send or the user send updated his
///   read mark: Notification counts
/// - EDUs that are active now (read receipts, typing updates, presence)
/// - TODO: Allow multiple sync streams to support Pantalaimon
///
/// For invited rooms:
/// - If the user was invited after `since`: A subset of the state of the room
///   at the point of the invite
///
/// For left rooms:
/// - If the user left after `since`: `prev_batch` token, empty state (TODO:
///   subset of the state at the point of the leave)
#[tracing::instrument(
    name = "sync",
    level = "debug",
    skip_all,
    fields(
        user_id = %body.sender_user(),
    )
)]
pub(crate) async fn sync_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<sync_events::v3::Request>,
) -> Result<sync_events::v3::Response> {
    let services = &*services;
    let sender_user = body.sender_user();
    let sender_device = body.sender_device.as_deref();

    // Presence update
    if services.config.client.allow_local_presence {
        services
            .presence
            .ping_presence(sender_user, &body.body.set_presence)
            .await
            .log_err()
            .ok();
    }

    let mut since = body
        .body
        .since
        .as_deref()
        .map(str::parse)
        .flat_ok()
        .unwrap_or(0);

    let timeout = body
        .body
        .timeout
        .as_ref()
        .map(Duration::as_millis)
        .map(TryInto::try_into)
        .flat_ok()
        .unwrap_or(services.config.client.client_sync_timeout_default)
        .max(services.config.client.client_sync_timeout_min)
        .min(services.config.client.client_sync_timeout_max);

    let stop_at = time::Instant::now()
        .checked_add(Duration::from_millis(timeout))
        .expect("configuration must limit maximum timeout");

    // The watcher keys to-device events by device; a deviceless appservice
    // user has none, so it watches an empty device ID that never matches.
    let watch_device = sender_device.unwrap_or_else(|| "".into());

    loop {
        // Poll the watcher once before sampling the count so the watches it
        // registers up front cannot miss a write made while building.
        let mut watchers = pin!(services.sync.watch(sender_user, watch_device).fuse());
        let fired = futures::poll!(watchers.as_mut()).is_ready();

        let next_batch = services.server_state.current_count();
        if since > next_batch {
            error!(since, next_batch, "received since > next_batch, clamping");
            since = next_batch;
        }

        if since < next_batch || body.body.full_state {
            let response = build_sync_events(services, &body, since, next_batch).await?;
            let empty = response.rooms.is_empty()
                && response.presence.is_empty()
                && response.account_data.is_empty()
                && response.device_lists.is_empty()
                && response.to_device.is_empty();

            if !empty || body.body.full_state {
                return Ok(response);
            }
        }

        // Wait for activity
        let woken = fired || time::timeout_at(stop_at, watchers).await.is_ok();
        if !woken || services.server.is_stopping() {
            trace!(since, next_batch, "empty response");
            return Ok(
                build_empty_response(services, sender_user, sender_device, next_batch).await,
            );
        }

        trace!(
            since,
            last_batch = ?next_batch,
            stop_at = ?stop_at,
            "notified by watcher"
        );

        since = next_batch;
    }
}

/// A response with no updates, still carrying the device's one-time key
/// counts so clients don't take a missing count for zero keys.
async fn build_empty_response(
    services: &Services,
    sender_user: &UserId,
    sender_device: Option<&DeviceId>,
    next_batch: u64,
) -> sync_events::v3::Response {
    let device_one_time_keys_count: OptionFuture<_> = sender_device
        .map(|sender_device| {
            services
                .users
                .count_one_time_keys(sender_user, sender_device)
        })
        .into();

    let mut response = sync_events::v3::Response::new(next_batch.to_string());
    response.device_one_time_keys_count = device_one_time_keys_count.await.unwrap_or_default();
    response
}

#[tracing::instrument(
    name = "build",
    level = "debug",
    ret(level = "trace"),
    skip_all,
    fields(
        %since,
        %next_batch,
    )
)]
async fn build_sync_events(
    services: &Services,
    body: &Ruma<sync_events::v3::Request>,
    since: u64,
    next_batch: u64,
) -> Result<sync_events::v3::Response> {
    let sender_user = body.sender_user();
    let sender_device = body.sender_device.as_deref();

    let full_state = body.body.full_state;
    let filter = match body.body.filter.as_ref() {
        Some(Filter::FilterDefinition(filter)) => filter.clone(),
        Some(Filter::FilterId(filter_id)) => services
            .users
            .get_filter(sender_user, filter_id)
            .await
            .unwrap_or_default(),
        _ => FilterDefinition::default(),
    };

    let joined_rooms = services
        .rooms
        .state_cache
        .rooms_joined(sender_user)
        .map(ToOwned::to_owned)
        .broad_filter_map(|room_id| {
            load_joined_room(
                services,
                sender_user,
                sender_device,
                room_id.clone(),
                since,
                next_batch,
                full_state,
                &filter,
            )
            .map_ok(move |(joined_room, dlu, jeu)| (room_id, joined_room, dlu, jeu))
            .ok()
        })
        .ready_fold(
            (BTreeMap::new(), HashSet::new(), HashSet::new()),
            |(mut joined_rooms, mut device_list_updates, mut left_encrypted_users),
             (room_id, joined_room, dlu, leu)| {
                device_list_updates.extend(dlu);
                left_encrypted_users.extend(leu);
                if !joined_room.is_empty() {
                    joined_rooms.insert(room_id, joined_room);
                }

                (joined_rooms, device_list_updates, left_encrypted_users)
            },
        );

    let left_rooms = services
        .rooms
        .state_cache
        .rooms_left(sender_user)
        .broad_filter_map(|(room_id, _)| {
            handle_left_room(
                services,
                since,
                room_id.clone(),
                sender_user,
                next_batch,
                full_state,
                &filter,
            )
            .map_ok(move |left_room| (room_id, left_room))
            .ok()
        })
        .ready_filter_map(|(room_id, left_room)| left_room.map(|left_room| (room_id, left_room)))
        .collect();

    let invited_rooms = services
        .rooms
        .state_cache
        .rooms_invited(sender_user)
        .fold_default(
            async |mut invited_rooms: BTreeMap<_, _>, (room_id, invite_state)| {
                let invite_count = services
                    .rooms
                    .state_cache
                    .get_invite_count(&room_id, sender_user)
                    .await
                    .ok();

                // Invited before last sync
                if Some(since) >= invite_count || Some(next_batch) < invite_count {
                    return invited_rooms;
                }

                let invited_room = InvitedRoom::from(InviteState::from(invite_state));

                invited_rooms.insert(room_id, invited_room);
                invited_rooms
            },
        );

    let knocked_rooms = services
        .rooms
        .state_cache
        .rooms_knocked(sender_user)
        .fold_default(
            async |mut knocked_rooms: BTreeMap<_, _>, (room_id, knock_state)| {
                let knock_count = services
                    .rooms
                    .state_cache
                    .get_knock_count(&room_id, sender_user)
                    .await
                    .ok();

                // Knocked before last sync; or after the cutoff for this sync
                if Some(since) >= knock_count || Some(next_batch) < knock_count {
                    return knocked_rooms;
                }

                let mut knock_state_events = KnockState::new();
                knock_state_events.events = knock_state;
                let knocked_room = KnockedRoom::from(knock_state_events);

                knocked_rooms.insert(room_id, knocked_room);
                knocked_rooms
            },
        );

    let presence_updates: OptionFuture<_> = services
        .config
        .client
        .allow_local_presence
        .then(|| process_presence_updates(services, since, next_batch, sender_user))
        .into();

    let account_data = services
        .account_data
        .changes_since(None, sender_user, since, Some(next_batch))
        .ready_filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Global))
        .collect();

    // Look for device list updates of this account
    let keys_changed = services
        .users
        .keys_changed(sender_user, since, Some(next_batch))
        .map(ToOwned::to_owned)
        .collect::<HashSet<_>>();

    let to_device_events: OptionFuture<_> = sender_device
        .map(|sender_device| {
            services
                .users
                .get_to_device_events(sender_user, sender_device, Some(since), Some(next_batch))
                .collect::<Vec<_>>()
        })
        .into();

    let device_one_time_keys_count: OptionFuture<_> = sender_device
        .map(|sender_device| {
            services
                .users
                .count_one_time_keys(sender_user, sender_device)
        })
        .into();

    // Remove all to-device events the device received *last time*
    let remove_to_device_events: OptionFuture<_> = sender_device
        .map(|sender_device| {
            services
                .users
                .remove_to_device_events(sender_user, sender_device, since)
        })
        .into();

    let (
        account_data,
        keys_changed,
        device_one_time_keys_count,
        (_, to_device_events, presence_updates),
        (
            (joined_rooms, mut device_list_updates, left_encrypted_users),
            left_rooms,
            invited_rooms,
            knocked_rooms,
        ),
    ) = join5(
        account_data,
        keys_changed,
        device_one_time_keys_count,
        join3(remove_to_device_events, to_device_events, presence_updates),
        join4(joined_rooms, left_rooms, invited_rooms, knocked_rooms),
    )
    .boxed()
    .await;

    device_list_updates.extend(keys_changed);

    // If the user doesn't share an encrypted room with the target anymore, we need
    // to tell them
    let device_list_left: HashSet<_> = left_encrypted_users
        .into_iter()
        .stream()
        .broad_filter_map(async |user_id: OwnedUserId| {
            (!share_encrypted_room(services, sender_user, &user_id, None).await).then_some(user_id)
        })
        .collect()
        .await;

    let presence_events = presence_updates
        .into_iter()
        .flat_map(IntoIterator::into_iter)
        .map(|(sender, content)| PresenceEvent { content, sender })
        .map(|ref event| Raw::new(event))
        .filter_map(Result::ok)
        .collect();

    let mut response = sync_events::v3::Response::new(next_batch.to_string());
    response.account_data = GlobalAccountData::new();
    response.account_data.events = account_data;
    response.device_lists = DeviceLists::new();
    response.device_lists.changed = device_list_updates.into_iter().collect();
    response.device_lists.left = device_list_left.into_iter().collect();
    response.device_one_time_keys_count = device_one_time_keys_count.unwrap_or_default();
    // Fallback keys are not yet supported
    response.device_unused_fallback_key_types = None;
    response.presence = Presence::new();
    response.presence.events = presence_events;
    response.rooms = Rooms::new();
    response.rooms.leave = left_rooms;
    response.rooms.join = joined_rooms;
    response.rooms.invite = invited_rooms;
    response.rooms.knock = knocked_rooms;
    response.to_device = ToDevice::new();
    response.to_device.events = to_device_events.unwrap_or_default();

    Ok(response)
}

#[tracing::instrument(name = "presence", level = "debug", skip_all)]
async fn process_presence_updates(
    services: &Services,
    since: u64,
    next_batch: u64,
    syncing_user: &UserId,
) -> PresenceUpdates {
    services
        .presence
        .presence_since(since)
        .filter_map(async |(user_id, count, presence_bytes)| {
            if count > next_batch {
                return None;
            }

            if !services
                .rooms
                .state_cache
                .user_sees_user(syncing_user, user_id)
                .await
            {
                return None;
            }

            let event = services
                .presence
                .from_json_bytes_to_event(presence_bytes, user_id)
                .await
                .ok()?;

            Some((user_id.to_owned(), event.content))
        })
        .collect()
        .await
}
