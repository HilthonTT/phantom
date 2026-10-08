use std::collections::{HashMap, HashSet};

use futures::{
    FutureExt, StreamExt, TryFutureExt, TryStreamExt,
    future::{OptionFuture, join, join3, join4, try_join3},
    pin_mut,
};
use phantom_core::{
    Result, at,
    bool::BoolExt,
    err, extract_variant,
    future::TryExt,
    is_equal_to,
    math::ruma_from_u64,
    matrix::{
        Event,
        pdu::{PduCount, PduEvent},
    },
    pair_of, ref_at,
    result::FlatOk,
    stream::{IterStream, ReadyExt, Tools, WidebandExt},
};
use phantom_service::{
    Services,
    accounts::account_data::AnyRawAccountDataEvent,
    rooms::{
        lazy_loading,
        lazy_loading::{Options, Witness},
        short::ShortStateHash,
    },
};
use ruma::{
    DeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId,
    api::client::{
        filter::FilterDefinition,
        sync::sync_events::v3::{JoinedRoom, State as RoomState, StateEvents},
    },
    events::{
        AnySyncEphemeralRoomEvent, StateEventType, SyncEphemeralRoomEvent,
        TimelineEventType::*,
        room::member::{MembershipState, RoomMemberEventContent},
        typing::TypingEventContent,
    },
    serde::Raw,
    uint,
};

use super::{
    super::{load_timeline, share_encrypted_room},
    state::{StateChanges, calculate_state_changes},
};
use crate::client::message::ignored_filter;

#[tracing::instrument(
    name = "joined",
    level = "debug",
    skip_all,
    fields(
        room_id = ?room_id,
    ),
)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn load_joined_room(
    services: &Services,
    sender_user: &UserId,
    sender_device: Option<&DeviceId>,
    room_id: OwnedRoomId,
    since: u64,
    next_batch: u64,
    full_state: bool,
    filter: &FilterDefinition,
) -> Result<(JoinedRoom, HashSet<OwnedUserId>, HashSet<OwnedUserId>)> {
    let room_id = &room_id;
    let since_shortstatehash = services
        .rooms
        .user
        .get_token_shortstatehash(room_id, since)
        .ok()
        .map(Ok);

    let timeline_limit: usize = filter
        .room
        .timeline
        .limit
        .unwrap_or_else(|| uint!(10))
        .try_into()?;

    let timeline = load_timeline(
        services,
        sender_user,
        room_id,
        PduCount::Normal(since),
        Some(PduCount::Normal(next_batch)),
        timeline_limit,
    );

    let receipt_events = services
        .rooms
        .read_receipt
        .readreceipts_since(room_id, since)
        .ready_filter(|&(_, count, _)| count <= next_batch)
        .filter_map(async |(read_user, _, edu)| {
            services
                .users
                .user_is_ignored(read_user, sender_user)
                .await
                .or_some((read_user.to_owned(), edu))
        })
        .collect::<HashMap<OwnedUserId, Raw<AnySyncEphemeralRoomEvent>>>()
        .map(Ok);

    let (since_shortstatehash, (timeline_pdus, limited, last_timeline_count), receipt_events) =
        try_join3(since_shortstatehash, timeline, receipt_events)
            .boxed()
            .await?;

    let horizon_shortstatehash: OptionFuture<_> = timeline_pdus
        .first()
        .map(ref_at!(1))
        .map(|pdu| {
            services
                .rooms
                .state_accessor
                .pdu_shortstatehash(&pdu.event_id)
        })
        .into();

    // The state at the newest event this sync covers; the room's current state
    // when that event has none recorded.
    let current_shortstatehash = shortstatehash_at_count(services, room_id, last_timeline_count)
        .or_else(|_| services.rooms.state.get_room_shortstatehash(room_id));

    let (horizon_shortstatehash, current_shortstatehash) =
        join(horizon_shortstatehash, current_shortstatehash)
            .boxed()
            .await;

    let current_shortstatehash = current_shortstatehash
        .map_err(|_| err!(Database(error!("Room {room_id} has no state"))))?;

    let associate_token = services.rooms.user.associate_token_shortstatehash(
        room_id,
        next_batch,
        current_shortstatehash,
    );

    let lazy_loading_enabled = filter.room.state.lazy_load_options.is_enabled()
        || filter.room.timeline.lazy_load_options.is_enabled();

    // Lazy-loading state is kept per device, so a deviceless appservice user
    // gets every member event instead.
    let lazy_loading_context = sender_device
        .filter(|_| lazy_loading_enabled)
        .map(|device_id| lazy_loading::Context {
            user_id: sender_user,
            device_id,
            room_id,
            token: Some(since),
            options: Some(&filter.room.state.lazy_load_options),
        });

    let initial = since == 0 || since_shortstatehash.is_none();

    // Reset lazy loading because this is an initial sync
    if let Some(lazy_loading_context) = lazy_loading_context.as_ref().filter(|_| initial) {
        services
            .rooms
            .lazy_loading
            .reset(lazy_loading_context)
            .await;
    }

    let witness: OptionFuture<_> = lazy_loading_context
        .as_ref()
        .map(|lazy_loading_context| {
            let witness: Witness = timeline_pdus
                .iter()
                .map(ref_at!(1))
                .map(Event::sender)
                .map(ToOwned::to_owned)
                .chain(receipt_events.keys().cloned())
                .collect();

            services
                .rooms
                .lazy_loading
                .witness_retain(witness, lazy_loading_context)
        })
        .into();

    let last_notification_read: OptionFuture<_> = timeline_pdus
        .is_empty()
        .then(|| {
            services
                .rooms
                .user
                .last_notification_read(sender_user, room_id)
        })
        .into();

    let since_sender_member: OptionFuture<_> = since_shortstatehash
        .map(|short| {
            services
                .rooms
                .state_accessor
                .state_get_content(short, &StateEventType::RoomMember, sender_user.as_str())
                .ok()
        })
        .into();

    let encrypted_room = services.rooms.state_accessor.is_encrypted_room(room_id);

    let last_privateread_update = services
        .rooms
        .read_receipt
        .last_privateread_update(sender_user, room_id);

    let (
        (witness, since_sender_member),
        (encrypted_room, ()),
        (last_privateread_update, last_notification_read),
    ) = join3(
        join(witness, since_sender_member),
        join(encrypted_room, associate_token),
        join(last_privateread_update, last_notification_read),
    )
    .boxed()
    .await;

    let joined_since_last_sync = since_sender_member
        .flatten()
        .is_none_or(|content: RoomMemberEventContent| content.membership != MembershipState::Join);

    let StateChanges {
        heroes,
        joined_member_count,
        invited_member_count,
        mut state_events,
    } = calculate_state_changes(
        services,
        sender_user,
        room_id,
        full_state,
        encrypted_room,
        since_shortstatehash,
        horizon_shortstatehash.flat_ok(),
        current_shortstatehash,
        joined_since_last_sync,
        witness.as_ref(),
    )
    .await?;

    let send_notification_counts =
        last_notification_read.is_none_or(|last_count| last_count.gt(&since));

    let is_sender_membership = |event: &PduEvent| {
        *event.event_type() == StateEventType::RoomMember.into()
            && event
                .state_key()
                .is_some_and(is_equal_to!(sender_user.as_str()))
    };

    let joined_sender_member: Option<_> = (joined_since_last_sync && timeline_pdus.is_empty())
        .then(|| {
            state_events
                .iter()
                .position(is_sender_membership)
                .map(|pos| state_events.swap_remove(pos))
        })
        .flatten();

    let notification_count: OptionFuture<_> = send_notification_counts
        .then(|| {
            services
                .rooms
                .user
                .notification_count(sender_user, room_id)
                .map(TryInto::try_into)
                .unwrap_or(uint!(0))
        })
        .into();

    let highlight_count: OptionFuture<_> = send_notification_counts
        .then(|| {
            services
                .rooms
                .user
                .highlight_count(sender_user, room_id)
                .map(TryInto::try_into)
                .unwrap_or(uint!(0))
        })
        .into();

    let private_read_event: OptionFuture<_> = last_privateread_update
        .gt(&since)
        .then(|| {
            services
                .rooms
                .read_receipt
                .private_read_get(room_id, sender_user)
                .map(Result::ok)
        })
        .into();

    let typing_events = services
        .rooms
        .typing
        .last_typing_update(room_id)
        .and_then(async |count| {
            if count <= since {
                return Ok(Vec::<Raw<AnySyncEphemeralRoomEvent>>::new());
            }

            let typings = typings_event_for_user(services, room_id, sender_user).await?;

            Ok(vec![serde_json::from_str(&serde_json::to_string(
                &typings,
            )?)?])
        })
        .unwrap_or(Vec::new());

    let extract_membership = |event: &PduEvent| {
        let content: RoomMemberEventContent = event.get_content().ok()?;
        let user_id: OwnedUserId = event.state_key()?.parse().ok()?;

        Some((content, user_id))
    };

    let timeline_membership_changes: Vec<_> = timeline_pdus
        .iter()
        .map(ref_at!(1))
        .filter(|_| !initial)
        .filter_map(extract_membership)
        .collect();

    let device_list_updates = state_events
        .iter()
        .stream()
        .ready_filter(|_| !initial)
        .ready_filter(|state_event| *state_event.event_type() == RoomMember)
        .ready_filter_map(extract_membership)
        .chain(timeline_membership_changes.into_iter().stream())
        .fold_default(
            async |(mut dlu, mut leu): pair_of!(HashSet<_>), (content, user_id)| {
                use MembershipState::*;

                let shares_encrypted_room = async |user_id| {
                    share_encrypted_room(services, sender_user, user_id, Some(room_id)).await
                };

                match content.membership {
                    Leave => leu.insert(user_id),
                    Join if joined_since_last_sync || !shares_encrypted_room(&user_id).await => {
                        dlu.insert(user_id)
                    }
                    _ => false,
                };

                (dlu, leu)
            },
        );

    let prev_batch = timeline_pdus.first().map(at!(0)).or_else(|| {
        joined_sender_member
            .is_some()
            .then_some(since)
            .map(Into::into)
    });

    let include_in_timeline = |event: &PduEvent| {
        let filter = &filter.room.timeline;
        event.matches(filter)
    };

    let room_events = timeline_pdus
        .into_iter()
        .stream()
        .wide_filter_map(|item| ignored_filter(services, item, sender_user))
        .map(at!(1))
        .chain(joined_sender_member.into_iter().stream())
        .ready_filter(include_in_timeline)
        .collect::<Vec<_>>();

    let device_updates = services
        .users
        .room_keys_changed(room_id, since, Some(next_batch))
        .map(|(user_id, _)| user_id)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    let account_data_events = services
        .account_data
        .changes_since(Some(room_id), sender_user, since, Some(next_batch))
        .ready_filter_map(|e| extract_variant!(e, AnyRawAccountDataEvent::Room))
        .collect();

    let (
        (notification_count, highlight_count),
        ((mut device_list_updates, left_encrypted_users), device_updates),
        (room_events, account_data_events, typing_events, private_read_event),
    ) = join3(
        join(notification_count, highlight_count),
        join(device_list_updates, device_updates),
        join4(
            room_events,
            account_data_events,
            typing_events,
            private_read_event,
        ),
    )
    .boxed()
    .await;

    device_list_updates.extend(device_updates);

    let is_in_timeline = |event: &PduEvent| {
        room_events
            .iter()
            .map(Event::event_id)
            .any(is_equal_to!(event.event_id()))
    };

    let include_in_state = |event: &PduEvent| {
        let filter = &filter.room.state;
        event.matches(filter) && (full_state || !is_in_timeline(event))
    };

    let state_events: Vec<_> = state_events
        .into_iter()
        .filter(include_in_state)
        .map(PduEvent::into_sync_state_event)
        .collect();

    let heroes: Vec<_> = heroes
        .into_iter()
        .flatten()
        .map(TryInto::try_into)
        .filter_map(Result::ok)
        .collect();

    let edus: Vec<Raw<AnySyncEphemeralRoomEvent>> = receipt_events
        .into_values()
        .chain(typing_events.into_iter())
        .chain(private_read_event.flatten().into_iter())
        .collect();

    let mut joined_room = JoinedRoom::new();
    joined_room.account_data.events = account_data_events;
    joined_room.ephemeral.events = edus;
    joined_room.state = RoomState::Before(StateEvents::from(state_events));
    joined_room.summary.joined_member_count = joined_member_count.map(ruma_from_u64);
    joined_room.summary.invited_member_count = invited_member_count.map(ruma_from_u64);
    joined_room.summary.heroes = heroes;
    joined_room.timeline.limited = limited || joined_since_last_sync;
    joined_room.timeline.prev_batch = prev_batch.as_ref().map(ToString::to_string);
    joined_room.timeline.events = room_events
        .into_iter()
        .map(PduEvent::into_sync_room_event)
        .collect();
    joined_room.unread_notifications.highlight_count = highlight_count;
    joined_room.unread_notifications.notification_count = notification_count;

    Ok((joined_room, device_list_updates, left_encrypted_users))
}

/// The state recorded at the room's newest event at or before `count`.
async fn shortstatehash_at_count(
    services: &Services,
    room_id: &RoomId,
    count: PduCount,
) -> Result<ShortStateHash> {
    let pdus = services
        .rooms
        .timeline
        .pdus_rev(None, room_id, Some(count.saturating_add(1)));

    pin_mut!(pdus);
    let (_, pdu) = pdus
        .try_next()
        .await?
        .ok_or_else(|| err!(Request(NotFound("Room {room_id} has no events"))))?;

    services
        .rooms
        .state_accessor
        .pdu_shortstatehash(&pdu.event_id)
        .await
}

async fn typings_event_for_user(
    services: &Services,
    room_id: &RoomId,
    sender_user: &UserId,
) -> Result<SyncEphemeralRoomEvent<TypingEventContent>> {
    let user_ids = services
        .rooms
        .typing
        .typing_users_for_user(room_id, sender_user)
        .await?;

    Ok(SyncEphemeralRoomEvent::new(TypingEventContent::new(
        user_ids,
    )))
}
