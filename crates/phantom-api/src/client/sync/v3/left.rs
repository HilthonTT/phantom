use std::collections::HashMap;

use futures::{FutureExt, StreamExt, TryFutureExt, TryStreamExt, pin_mut};
use phantom_core::{
    Result, error,
    future::BoolExt as FutureBoolExt,
    is_equal_to,
    matrix::pdu::{EventHash, PduEvent},
    rand,
    time::now_millis,
    warn,
};
use phantom_service::{Services, rooms::lazy_loading::Options};
use ruma::{
    EventId, OwnedEventId, OwnedRoomId, UserId,
    api::client::{
        filter::FilterDefinition,
        sync::sync_events::v3::{LeftRoom, State as RoomState, StateEvents},
    },
    events::{StateEventType, TimelineEventType::*},
    uint,
};

#[tracing::instrument(
    name = "left",
    level = "debug",
    skip_all,
    fields(
        room_id = %room_id,
        full = %full_state,
    ),
)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_left_room(
    services: &Services,
    since: u64,
    room_id: OwnedRoomId,
    sender_user: &UserId,
    next_batch: u64,
    full_state: bool,
    filter: &FilterDefinition,
) -> Result<Option<LeftRoom>> {
    let room_id = &room_id;
    let left_count = services
        .rooms
        .state_cache
        .get_left_count(room_id, sender_user)
        .await
        .ok();

    let filter_exclude = filter.room.not_rooms.iter().any(is_equal_to!(room_id));

    let filter_include = filter
        .room
        .rooms
        .as_ref()
        .is_some_and(|rooms| rooms.iter().any(is_equal_to!(room_id)));

    let too_soon = Some(next_batch) < left_count;
    let too_late = Some(since) >= left_count;
    let initial_sync = since == 0;
    let include_leave =
        filter.room.include_leave && !filter_exclude && (filter_include || initial_sync);

    // Left before last sync or after cutoff for next sync
    if (too_late && !include_leave) || too_soon {
        return Ok(None);
    }

    let is_not_found = services
        .rooms
        .metadata
        .exists(room_id)
        .map(|exists| !exists);

    let is_disabled = services.rooms.metadata.is_disabled(room_id);

    let is_banned = services.rooms.metadata.is_banned(room_id);

    pin_mut!(is_not_found, is_disabled, is_banned);
    if is_not_found.or(is_disabled).or(is_banned).await {
        // This is just a rejected invite, not a room we know
        // Insert a leave event anyways for the client
        let event = PduEvent {
            event_id: EventId::parse(format!("${}", rand::string(43)))?,
            sender: sender_user.to_owned(),
            origin: None,
            origin_server_ts: now_millis().try_into()?,
            kind: RoomMember,
            content: serde_json::from_str(r#"{"membership":"leave"}"#)?,
            state_key: Some(sender_user.as_str().into()),
            unsigned: None,
            // The following keys are dropped on conversion
            room_id: room_id.clone(),
            prev_events: vec![],
            depth: uint!(1),
            auth_events: vec![],
            redacts: None,
            hashes: EventHash {
                sha256: String::new(),
            },
            signatures: None,
        };

        let mut left_room = LeftRoom::new();
        left_room.timeline.prev_batch = Some(next_batch.to_string());
        left_room.state = RoomState::Before(StateEvents::from(vec![event.into_sync_state_event()]));

        return Ok(Some(left_room));
    }

    let mut left_state_events = Vec::new();

    let since_shortstatehash = services.rooms.user.get_token_shortstatehash(room_id, since);

    let since_state_ids: HashMap<_, OwnedEventId> = since_shortstatehash
        .map_ok(|since_shortstatehash| {
            services
                .rooms
                .state_accessor
                .state_full_ids(since_shortstatehash)
                .map(Ok)
        })
        .try_flatten_stream()
        .try_collect()
        .await
        .unwrap_or_default();

    let Ok(left_event_id): Result<OwnedEventId> = services
        .rooms
        .state_accessor
        .room_state_get_id(room_id, &StateEventType::RoomMember, sender_user.as_str())
        .await
    else {
        warn!("Left {room_id} but no left state event");
        return Ok(None);
    };

    let Ok(left_shortstatehash) = services
        .rooms
        .state_accessor
        .pdu_shortstatehash(&left_event_id)
        .await
    else {
        warn!(event_id = %left_event_id, "Leave event has no state in {room_id}");
        return Ok(None);
    };

    let mut left_state_ids: HashMap<_, _> = services
        .rooms
        .state_accessor
        .state_full_ids(left_shortstatehash)
        .collect()
        .await;

    let leave_shortstatekey = services
        .rooms
        .short
        .get_or_create_shortstatekey(&StateEventType::RoomMember, sender_user.as_str())
        .await;

    left_state_ids.insert(leave_shortstatekey, left_event_id);

    for (shortstatekey, event_id) in left_state_ids {
        if full_state || since_state_ids.get(&shortstatekey) != Some(&event_id) {
            let (event_type, state_key) = services
                .rooms
                .short
                .get_statekey_from_short(shortstatekey)
                .await?;

            if filter.room.state.lazy_load_options.is_enabled()
                && event_type == StateEventType::RoomMember
                && !full_state
                && state_key
                    .as_str()
                    .try_into()
                    .is_ok_and(|user_id: &UserId| sender_user != user_id)
            {
                continue;
            }

            let Ok(pdu) = services.rooms.timeline.get_pdu(&event_id).await else {
                error!("Pdu in state not found: {event_id}");
                continue;
            };

            left_state_events.push(pdu.into_sync_state_event());
        }
    }

    let mut left_room = LeftRoom::new();
    // TODO: support left timeline events so limited need not be set
    left_room.timeline.limited = true;
    left_room.timeline.prev_batch = Some(next_batch.to_string());
    left_room.state = RoomState::Before(StateEvents::from(left_state_events));

    Ok(Some(left_room))
}
