use futures::{
    FutureExt, StreamExt,
    future::{OptionFuture, join},
};
use phantom_core::{
    Result,
    future::{OptionStream, TryExt},
    is_equal_to,
    matrix::pdu::PduEvent,
    result::FlatOk,
    stream::{BroadbandExt, IterStream, ReadyExt, Tools, TryExpect},
};
use phantom_service::{
    Services,
    rooms::{
        lazy_loading::Witness,
        short::{ShortEventId, ShortStateHash, ShortStateKey},
    },
};
use ruma::{
    OwnedEventId, OwnedUserId, RoomId, UserId,
    events::{
        StateEventType,
        TimelineEventType::*,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};

#[derive(Default)]
pub(super) struct StateChanges {
    pub(super) heroes: Option<Vec<OwnedUserId>>,
    pub(super) joined_member_count: Option<u64>,
    pub(super) invited_member_count: Option<u64>,
    pub(super) state_events: Vec<PduEvent>,
}

#[tracing::instrument(
    name = "state",
    level = "trace",
    skip_all,
    fields(
        full = %full_state,
        cs = %current_shortstatehash,
        ss = ?since_shortstatehash,
    )
)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn calculate_state_changes<'a>(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    full_state: bool,
    encrypted_room: bool,
    since_shortstatehash: Option<ShortStateHash>,
    horizon_shortstatehash: Option<ShortStateHash>,
    current_shortstatehash: ShortStateHash,
    joined_since_last_sync: bool,
    witness: Option<&'a Witness>,
) -> Result<StateChanges> {
    let initial = full_state || since_shortstatehash.is_none() || joined_since_last_sync;

    let incremental = !initial && since_shortstatehash != Some(current_shortstatehash);

    let horizon_shortstatehash = horizon_shortstatehash.unwrap_or(current_shortstatehash);

    let since_shortstatehash = since_shortstatehash.unwrap_or(horizon_shortstatehash);

    let state_get_shorteventid = |user_id: &'a UserId| {
        services
            .rooms
            .state_accessor
            .state_get_shortid(
                horizon_shortstatehash,
                &StateEventType::RoomMember,
                user_id.as_str(),
            )
            .ok()
    };

    let lazy_state_ids: OptionFuture<_> = witness
        .filter(|_| !encrypted_room)
        .map(|witness| {
            StreamExt::into_future(
                witness
                    .iter()
                    .stream()
                    .broad_filter_map(|user_id| state_get_shorteventid(user_id)),
            )
        })
        .into();

    let state_diff_ids: OptionFuture<_> = incremental
        .then(|| {
            StreamExt::into_future(
                services
                    .rooms
                    .state_accessor
                    .state_added((since_shortstatehash, horizon_shortstatehash))
                    .boxed(),
            )
        })
        .into();

    let current_state_ids: OptionFuture<_> = initial
        .then(|| {
            StreamExt::into_future(
                services
                    .rooms
                    .state_accessor
                    .state_full_shortids(horizon_shortstatehash)
                    .expect_ok(),
            )
        })
        .into();

    let state_events = current_state_ids
        .stream()
        .chain(state_diff_ids.stream())
        .broad_filter_map(async |(shortstatekey, shorteventid)| {
            if witness.is_none() || encrypted_room {
                return Some(shorteventid);
            }

            lazy_filter(services, sender_user, shortstatekey, shorteventid).await
        })
        .chain(lazy_state_ids.stream())
        .broad_filter_map(|shorteventid| {
            services
                .rooms
                .short
                .get_eventid_from_short(shorteventid)
                .ok()
        })
        .broad_filter_map(async |event_id: OwnedEventId| {
            services.rooms.timeline.get_pdu(&event_id).ok().await
        })
        .collect::<Vec<_>>()
        .boxed()
        .await;

    let send_member_counts = state_events.iter().any(|event| event.kind == RoomMember);

    let member_counts: OptionFuture<_> = send_member_counts
        .then(|| calculate_counts(services, room_id, sender_user))
        .into();

    let (joined_member_count, invited_member_count, heroes) =
        member_counts.await.unwrap_or((None, None, None));

    Ok(StateChanges {
        heroes,
        joined_member_count,
        invited_member_count,
        state_events,
    })
}

async fn lazy_filter(
    services: &Services,
    sender_user: &UserId,
    shortstatekey: ShortStateKey,
    shorteventid: ShortEventId,
) -> Option<ShortEventId> {
    let (event_type, state_key) = services
        .rooms
        .short
        .get_statekey_from_short(shortstatekey)
        .await
        .ok()?;

    (event_type != StateEventType::RoomMember || state_key == sender_user.as_str())
        .then_some(shorteventid)
}

async fn calculate_counts(
    services: &Services,
    room_id: &RoomId,
    sender_user: &UserId,
) -> (Option<u64>, Option<u64>, Option<Vec<OwnedUserId>>) {
    let joined_member_count = services
        .rooms
        .state_cache
        .room_joined_count(room_id)
        .unwrap_or(0);

    let invited_member_count = services
        .rooms
        .state_cache
        .room_invited_count(room_id)
        .unwrap_or(0);

    let (joined_member_count, invited_member_count) =
        join(joined_member_count, invited_member_count).await;

    let small_room = joined_member_count.saturating_add(invited_member_count) <= 5;

    let heroes: OptionFuture<_> = small_room
        .then(|| calculate_heroes(services, room_id, sender_user))
        .into();

    (
        Some(joined_member_count),
        Some(invited_member_count),
        heroes.await,
    )
}

async fn calculate_heroes(
    services: &Services,
    room_id: &RoomId,
    sender_user: &UserId,
) -> Vec<OwnedUserId> {
    services
        .rooms
        .timeline
        .all_pdus(sender_user, room_id)
        .ready_filter(|(_, pdu)| pdu.kind == RoomMember)
        .fold_default(|heroes: Vec<_>, (_, pdu)| {
            fold_hero(heroes, services, room_id, sender_user, pdu)
        })
        .await
}

async fn fold_hero(
    mut heroes: Vec<OwnedUserId>,
    services: &Services,
    room_id: &RoomId,
    sender_user: &UserId,
    pdu: PduEvent,
) -> Vec<OwnedUserId> {
    let Some(user_id): Option<&UserId> = pdu.state_key.as_deref().map(TryInto::try_into).flat_ok()
    else {
        return heroes;
    };

    if user_id == sender_user {
        return heroes;
    }

    let Ok(content): Result<RoomMemberEventContent, _> = pdu.get_content() else {
        return heroes;
    };

    // The membership was and still is invite or join
    if !matches!(
        content.membership,
        MembershipState::Join | MembershipState::Invite
    ) {
        return heroes;
    }

    if heroes.iter().any(is_equal_to!(user_id)) {
        return heroes;
    }

    let (is_invited, is_joined) = join(
        services.rooms.state_cache.is_invited(user_id, room_id),
        services.rooms.state_cache.is_joined(user_id, room_id),
    )
    .await;

    if !is_joined && is_invited {
        return heroes;
    }

    heroes.push(user_id.to_owned());
    heroes
}
