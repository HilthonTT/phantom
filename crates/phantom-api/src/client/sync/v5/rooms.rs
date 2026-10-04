mod bump_stamp;
mod heroes;

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use futures::{
    FutureExt, StreamExt, TryFutureExt, TryStreamExt,
    future::{OptionFuture, join, join3, join4},
    pin_mut,
};
use phantom_core::{
    Error, Result,
    hash::sha256,
    matrix::{Event, PduCount, PduEvent, StateKey},
    stream::{BroadbandExt, IterStream, WidebandExt},
};
use phantom_service::{Services, rooms::short::ShortStateHash};
use ruma::{
    JsOption, MxcUri, OwnedMxcUri, RoomId, UInt, UserId,
    api::client::sync::sync_events::{UnreadNotificationsCount, v5::response},
    events::{AnySyncStateEvent, StateEventType, TimelineEventType, room::member::MembershipState},
    serde::Raw,
};

use self::{bump_stamp::room_bump_stamp, heroes::calculate_heroes};
use super::{
    super::load_timeline,
    ListIds, SyncInfo, WindowRoom,
    connection::{Connection, Room},
};
use crate::client::{annotate_membership, message::ignored_filter, with_membership};

#[derive(Debug)]
pub(super) enum Failure {
    Timeline(Error),
    Payload(Error),
}

pub(super) type RoomDetails = (usize, HashSet<(StateEventType, StateKey)>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StateMode {
    Full,
    Delta(PduCount),
}

#[tracing::instrument(name = "room", level = "debug", skip_all, fields(room_id, roomsince))]
pub(super) async fn handle_room(
    sync_info: SyncInfo<'_>,
    conn: &Connection,
    window_room: &WindowRoom,
    room: &Room,
    config_changed: bool,
    room_details: RoomDetails,
) -> Result<response::Room, Failure> {
    let SyncInfo {
        services,
        sender_user,
        previous_connection_pos,
        direct_rooms,
        ..
    } = sync_info;

    let WindowRoom {
        membership,
        room_id,
        ..
    } = window_room;
    let roomsince = room.roomsince;

    if matches!(
        *membership,
        Some(MembershipState::Leave | MembershipState::Ban)
    ) {
        return leave_or_ban_response(sync_info, conn, window_room, roomsince)
            .map_err(Failure::Payload)
            .await;
    }

    let is_invite = *membership == Some(MembershipState::Invite);

    let encrypted = services.rooms.state_accessor.is_encrypted_room(room_id);

    let (timeline_limit, required_state) = room_details;

    let timeline: OptionFuture<_> = (!is_invite)
        .then(|| {
            load_timeline(
                services,
                sender_user,
                room_id,
                PduCount::Normal(roomsince),
                Some(PduCount::from(conn.next_batch)),
                timeline_limit,
            )
        })
        .into();

    let timeline = timeline.map(Option::transpose).map_err(Failure::Timeline);

    let (encrypted, timeline) = join(encrypted, timeline).await;

    // A failed load must fail the room, else roomsince advances past unsent events.
    let (timeline_pdus, limited, last_timeline_count) =
        timeline?.unwrap_or_else(|| (Vec::new(), false, PduCount::default()));

    let limited = room_timeline_limited(timeline_limit, limited);

    let prev_batch = timeline_pdus
        .first()
        .map(|(count, _)| count.into_unsigned().to_string());

    let bump_stamp = room_bump_stamp(
        services,
        sender_user,
        room_id,
        PduCount::Normal(roomsince),
        PduCount::from(conn.next_batch),
        last_timeline_count,
    )
    .map_err(Failure::Timeline)
    .await?;

    // phantom stores only a fingerprint of the delivered configuration, so a
    // changed configuration resends the whole required state rather than just
    // the newly requested entries.
    let mode = state_mode(roomsince, config_changed);
    let changed = state_may_have_changed(mode, last_timeline_count);

    let required_state = if membership_allows_required_state(membership.as_ref()) && changed {
        required_state
    } else {
        HashSet::new()
    };

    let required_state = collect_required_state(
        services,
        sender_user,
        room_id,
        mode,
        &required_state,
        &timeline_pdus,
        encrypted,
    );

    // TODO: figure out a timestamp we can use for remote invites
    let invite_state: OptionFuture<_> = is_invite
        .then(|| {
            services
                .rooms
                .state_cache
                .invite_state(sender_user, room_id)
                .map(Result::ok)
        })
        .into();

    let timeline = timeline_pdus
        .iter()
        .stream()
        .filter_map(|item| ignored_filter(services, item.clone(), sender_user))
        .wide_then(|(position, pdu)| {
            with_membership(services, pdu, sender_user, encrypted).map(move |pdu| (position, pdu))
        })
        .map(|(position, pdu)| (position, pdu.into_sync_room_event()))
        .collect::<Vec<_>>();

    let meta = room_meta_future(services, room_id);
    let events = join3(timeline, required_state, invite_state);
    let member_counts = member_counts_future(services, room_id);
    let notification_counts = notification_counts_future(services, sender_user, room_id);
    let (
        (room_name, room_avatar),
        (timeline, required_state, invite_state),
        (joined_count, invited_count),
        unread_notifications,
    ) = join4(meta, events, member_counts, notification_counts)
        .boxed()
        .await;

    let (heroes, heroes_name, heroes_avatar) = resolve_heroes(
        services,
        sender_user,
        room_id,
        room_name.as_deref(),
        room_avatar.as_deref(),
    )
    .await;

    let previous_connection_pos = previous_connection_pos.filter(|_| !is_invite);
    let (initial, num_live) = room_timeline_metadata(roomsince, previous_connection_pos, &timeline);

    let mut response = response::Room::new();
    response.initial = initial;
    response.name = room_name.or(heroes_name);
    response.avatar = JsOption::from_option(room_avatar.or(heroes_avatar));
    response.is_dm = direct_rooms.contains(room_id).then_some(true);
    response.heroes = heroes;
    response.required_state = required_state;
    response.invite_state = invite_state.flatten();
    response.prev_batch = prev_batch;
    response.num_live = num_live;
    response.limited = limited;
    response.timeline = timeline.into_iter().map(|(_, event)| event).collect();
    response.bump_stamp = bump_stamp;
    response.joined_count = joined_count;
    response.invited_count = invited_count;
    response.unread_notifications = unread_notifications;

    Ok(response)
}

async fn leave_or_ban_response(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    WindowRoom { room_id, .. }: &WindowRoom,
    roomsince: u64,
) -> Result<response::Room> {
    // A rejected federated invite has no resolved state; the retraction still
    // delivers on the membership alone.
    let member_event = services
        .rooms
        .state_accessor
        .room_state_get(room_id, &StateEventType::RoomMember, sender_user.as_str())
        .await
        .ok()
        .map(PduEvent::into_sync_state_event);

    let mut response = response::Room::new();
    response.initial = roomsince.eq(&0).then_some(true);
    response.prev_batch = Some(conn.next_batch.to_string());
    response.limited = true;
    response.required_state = member_event.into_iter().collect();

    Ok(response)
}

pub(super) fn merged_room_details(
    conn: &Connection,
    lists: &ListIds,
    room_id: &RoomId,
) -> RoomDetails {
    let lists = lists
        .iter()
        .filter_map(|list_id| conn.lists.get(list_id))
        .map(|list| {
            (
                &list.room_details.required_state,
                list.room_details.timeline_limit,
            )
        });

    let subscription = conn
        .subscriptions
        .get(room_id)
        .map(|config| (&config.required_state, config.timeline_limit));

    lists.chain(subscription).fold(
        (0_usize, HashSet::new()),
        |(timeline_limit, mut required_state), (config_state, config_limit)| {
            required_state.extend(config_state.iter().map(|(event_type, state_key)| {
                (event_type.clone(), StateKey::from_str(state_key))
            }));

            let config_limit = usize::try_from(u64::from(config_limit)).unwrap_or(usize::MAX);
            (timeline_limit.max(config_limit), required_state)
        },
    )
}

/// Fingerprints a room's delivered configuration.
///
/// The fingerprint is order-independent over the required-state selectors.
pub(super) fn room_config((timeline_limit, required_state): &RoomDetails) -> u64 {
    let timeline_limit = u64::try_from(*timeline_limit).expect("timeline limit must fit u64");
    let digest = sha256::hash(timeline_limit.to_be_bytes());

    required_state
        .iter()
        .fold(digest_word(&digest), |hash, (event_type, state_key)| {
            hash ^ required_state_hash(event_type, state_key.as_str())
        })
}

fn state_mode(roomsince: u64, config_changed: bool) -> StateMode {
    match (roomsince, config_changed) {
        (0, _) | (_, true) => StateMode::Full,
        (roomsince, false) => StateMode::Delta(PduCount::Normal(roomsince)),
    }
}

fn required_state_hash(event_type: &StateEventType, state_key: &str) -> u64 {
    let event_type = event_type.to_string();
    let digest = sha256::delimited([event_type.as_str(), state_key].into_iter());

    digest_word(&digest)
}

fn digest_word(digest: &[u8]) -> u64 {
    u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("SHA-256 digest must contain eight bytes"),
    )
}

pub(super) fn membership_allows_required_state(membership: Option<&MembershipState>) -> bool {
    matches!(membership, None | Some(MembershipState::Join))
}

/// Whether a room's newest event lies past the delta cursor.
///
/// State changes arrive as timeline events, so a room with nothing newer than
/// the cursor has none to report. A full sync always reports.
fn state_may_have_changed(state_mode: StateMode, last_timeline_count: PduCount) -> bool {
    match state_mode {
        StateMode::Full => true,
        StateMode::Delta(since) => last_timeline_count > since,
    }
}

fn room_timeline_limited(timeline_limit: usize, limited: bool) -> bool {
    timeline_limit > 0 && limited
}

fn room_timeline_metadata<Event>(
    roomsince: u64,
    previous_connection_pos: Option<u64>,
    timeline_pdus: &[(PduCount, Event)],
) -> (Option<bool>, Option<UInt>) {
    let initial = roomsince.eq(&0).then_some(true);
    let num_live =
        previous_connection_pos
            .map(PduCount::from)
            .and_then(|previous_connection_pos| {
                timeline_pdus
                    .iter()
                    .rev()
                    .map(|(position, _)| *position)
                    .take_while(|position| *position > previous_connection_pos)
                    .count()
                    .try_into()
                    .ok()
            });

    (initial, num_live)
}

async fn resolve_heroes(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    room_name: Option<&str>,
    room_avatar: Option<&MxcUri>,
) -> (
    Option<Vec<response::Hero>>,
    Option<String>,
    Option<OwnedMxcUri>,
) {
    if !services.config.client.calculate_heroes {
        return Default::default();
    }

    calculate_heroes(services, sender_user, room_id, room_name, room_avatar).await
}

async fn room_meta_future(
    services: &Services,
    room_id: &RoomId,
) -> (Option<String>, Option<OwnedMxcUri>) {
    let state_accessor = &services.rooms.state_accessor;
    let room_name = state_accessor.get_name(room_id).map(Result::ok);

    let room_avatar = state_accessor
        .get_avatar(room_id)
        .map(|content| content.into_option().and_then(|content| content.url));

    join(room_name, room_avatar).await
}

async fn member_counts_future(
    services: &Services,
    room_id: &RoomId,
) -> (Option<UInt>, Option<UInt>) {
    let state_cache = &services.rooms.state_cache;
    let joined_count = state_cache
        .room_joined_count(room_id)
        .map(|count| count.ok().and_then(|count| count.try_into().ok()));

    let invited_count = state_cache
        .room_invited_count(room_id)
        .map(|count| count.ok().and_then(|count| count.try_into().ok()));

    join(joined_count, invited_count).await
}

/// The room's unread counts.
///
/// phantom keeps no per-thread notification counts, so the room totals stand
/// alone.
async fn notification_counts_future(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
) -> UnreadNotificationsCount {
    let user = &services.rooms.user;
    let (highlight_count, notification_count) = join(
        user.highlight_count(sender_user, room_id),
        user.notification_count(sender_user, room_id),
    )
    .await;

    let mut counts = UnreadNotificationsCount::new();
    counts.highlight_count = highlight_count.try_into().ok();
    counts.notification_count = notification_count.try_into().ok();
    counts
}

/// The room's state as of `since`: the state after its last event at or
/// before that position.
pub(super) async fn shortstatehash_at(
    services: &Services,
    room_id: &RoomId,
    since: PduCount,
) -> Option<ShortStateHash> {
    let pdus = services
        .rooms
        .timeline
        .pdus_rev(None, room_id, Some(since.saturating_add(1)));

    pin_mut!(pdus);
    let (_, pdu) = pdus.try_next().await.ok()??;

    services
        .rooms
        .state_accessor
        .pdu_shortstatehash(pdu.event_id())
        .await
        .ok()
}

async fn collect_required_state(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    state_mode: StateMode,
    required_state: &HashSet<(StateEventType, StateKey)>,
    timeline_pdus: &[(PduCount, PduEvent)],
    encrypted: bool,
) -> Vec<Raw<AnySyncStateEvent>> {
    let state_accessor = &services.rooms.state_accessor;
    let lazy = required_state.iter().any(|(event_type, state_key)| {
        *event_type == StateEventType::RoomMember && state_key == "$LAZY"
    });

    let needs_since_state = required_state
        .iter()
        .any(|(_, state_key)| state_key != "$LAZY");

    let current_shortstatehash = services
        .rooms
        .state
        .get_room_shortstatehash(room_id)
        .await
        .ok();

    // Falling back to current state would match every entry against itself.
    let since_state = match state_mode {
        StateMode::Delta(since) if needs_since_state => shortstatehash_at(services, room_id, since)
            .await
            .map(|shortstatehash| (since, shortstatehash)),
        _ => None,
    };

    // Equal hashes exclude changes.
    let state_unchanged =
        since_state.is_some_and(|(_, since)| current_shortstatehash == Some(since));

    let timeline_senders = timeline_pdus
        .iter()
        .filter(|_| lazy)
        .map(|(_, pdu)| pdu.sender().as_str());

    let timeline_member_targets = timeline_pdus
        .iter()
        .filter(|_| lazy)
        .map(|(_, pdu)| pdu)
        .filter(|event| *event.event_type() == TimelineEventType::RoomMember)
        .filter_map(Event::state_key);

    let wildcard_state = required_state
        .iter()
        .filter(|(_, state_key)| !state_unchanged && state_key == "*")
        .filter_map(|(event_type, _)| current_shortstatehash.map(|hash| (event_type, hash)))
        .stream()
        .flat_map(|(event_type, shortstatehash)| {
            state_accessor
                .state_keys_with_ids(shortstatehash, event_type)
                .map(move |(state_key, event_id)| {
                    ((event_type.clone(), state_key), Some(event_id), false)
                })
        });

    let mut timeline_members: Vec<&str> = timeline_senders.chain(timeline_member_targets).collect();

    timeline_members.sort_unstable();
    timeline_members.dedup();

    let timeline_members = timeline_members
        .into_iter()
        .map(|sender| (StateEventType::RoomMember, StateKey::from_str(sender)));

    required_state
        .iter()
        .filter(|_| !state_unchanged)
        .cloned()
        .map(|state| (state, None, false))
        .stream()
        .chain(wildcard_state)
        .chain(timeline_members.map(|state| (state, None, true)).stream())
        .broad_filter_map(async |(state, event_id, lazy)| {
            let (event_type, state_key) = state;
            let state_key: StateKey = match state_key.as_str() {
                "$LAZY" | "*" => return None,
                "$ME" => StateKey::from_str(sender_user.as_str()),
                _ => state_key,
            };

            let event_id: ruma::OwnedEventId = match event_id {
                Some(event_id) => event_id,
                None => state_accessor
                    .room_state_get_id(room_id, &event_type, &state_key)
                    .await
                    .ok()?,
            };

            let pdu_id = services.rooms.timeline.get_pdu_id(&event_id).await.ok();
            let count = pdu_id.map(|pdu_id| pdu_id.pdu_count());

            let same_at_since = match since_state {
                Some((since, shortstatehash))
                    if !lazy && count.is_some_and(|count| count <= since) =>
                {
                    state_accessor
                        .state_get_id::<ruma::OwnedEventId>(shortstatehash, &event_type, &state_key)
                        .await
                        .is_ok_and(|previous_event_id| previous_event_id == event_id)
                }
                _ => false,
            };

            if !state_is_required(state_mode, count, lazy, same_at_since) {
                return None;
            }

            let mut pdu = match pdu_id {
                None => services
                    .rooms
                    .outlier
                    .get_pdu_outlier(&event_id)
                    .await
                    .ok()?,
                Some(pdu_id) => services
                    .rooms
                    .timeline
                    .get_pdu_from_id(&pdu_id)
                    .or_else(|_| services.rooms.outlier.get_pdu_outlier(&event_id))
                    .await
                    .ok()?,
            };

            annotate_membership(services, &mut pdu, sender_user, encrypted).await;

            Some(pdu.into_sync_state_event())
        })
        .collect()
        .await
}

fn state_is_required(
    state_mode: StateMode,
    count: Option<PduCount>,
    lazy: bool,
    same_at_since: bool,
) -> bool {
    lazy || match state_mode {
        StateMode::Full => true,
        StateMode::Delta(since) => count.is_none_or(|count| count > since) || !same_at_since,
    }
}
