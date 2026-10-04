use axum::extract::State;
use futures::{FutureExt, StreamExt, TryFutureExt, pin_mut};
use phantom_core::{
    Err, Result, at,
    bool::BoolExt,
    err,
    math::usize_from_ruma_bounded,
    matrix::{
        event::Event,
        pdu::{PduCount, PduEvent},
    },
    ref_at,
    result::LogErr,
    stream::{BroadbandExt, IterStream, ReadyExt, TryIgnore, WidebandExt},
};
use phantom_service::{
    Services,
    ops::moderation::Restriction,
    rooms::{
        lazy_loading,
        lazy_loading::{Options, Witness},
        timeline::PdusIterItem,
    },
};
use ruma::{
    DeviceId, RoomId, UInt, UserId,
    api::{
        Direction,
        client::{filter::RoomEventFilter, message::get_message_events},
    },
    events::{
        AnyStateEvent, StateEventType, TimelineEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
    serde::Raw,
};
use serde_json::value::to_raw_value;

use super::visibility_filter;
use crate::router::Ruma;

/// Shared inputs for [`get_messages`], the pagination core behind both the
/// client-server `/messages` route and the admin room-messages endpoint.
pub(crate) struct MessagesArgs<'a> {
    pub room_id: &'a RoomId,
    pub sender_user: &'a UserId,
    pub sender_device: Option<&'a DeviceId>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub dir: Direction,
    pub limit: Option<UInt>,
    pub filter: &'a RoomEventFilter,

    /// Skip the room-visibility gate and the per-event visibility and ignore
    /// filters, for admin callers that see all history.
    pub bypass_visibility: bool,
}

/// list of safe and common non-state events to ignore if the user is ignored.
const IGNORED_MESSAGE_TYPES: &[TimelineEventType] = &[
    TimelineEventType::CallInvite,
    TimelineEventType::KeyVerificationStart,
    TimelineEventType::Reaction,
    TimelineEventType::RoomEncrypted,
    TimelineEventType::RoomMessage,
    TimelineEventType::Sticker,
];

const LIMIT_MAX: usize = 1000;
const LIMIT_DEFAULT: usize = 10;

/// # `GET /_matrix/client/r0/rooms/{roomId}/messages`
///
/// Allows paginating through room history.
///
/// - Only works if the user is joined (TODO: always allow, but only show events
///   where the user was joined, depending on `history_visibility`)
pub(crate) async fn get_message_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_message_events::v3::Request>,
) -> Result<get_message_events::v3::Response> {
    get_messages(
        &services,
        MessagesArgs {
            room_id: &body.room_id,
            sender_user: body.sender_user(),
            sender_device: body.sender_device.as_deref(),
            from: body.from.as_deref(),
            to: body.to.as_deref(),
            dir: body.dir,
            limit: Some(body.limit),
            filter: &body.filter,
            bypass_visibility: false,
        },
    )
    .await
}

/// Paginates a room's timeline, applying the request filter and (unless
/// `bypass_visibility`) the per-user visibility and ignore filters. Powers the
/// client-server `/messages` route and its admin bypass twin.
pub(crate) async fn get_messages(
    services: &Services,
    args: MessagesArgs<'_>,
) -> Result<get_message_events::v3::Response> {
    let MessagesArgs {
        room_id,
        sender_user,
        sender_device,
        from,
        to,
        dir,
        limit,
        filter,
        bypass_visibility,
    } = args;

    if !services.rooms.metadata.exists(room_id).await {
        return Err!(Request(Forbidden("Room does not exist to this server")));
    }

    if !bypass_visibility && !user_can_see_room(services, sender_user, room_id).await {
        return Err!(Request(Forbidden(
            "You don't have permission to view this room."
        )));
    }

    let from: PduCount = from
        .map(str::parse)
        .transpose()
        .map_err(|_| err!(Request(InvalidParam("Invalid `from` token."))))?
        .unwrap_or_else(|| match dir {
            Direction::Forward => PduCount::min(),
            Direction::Backward => PduCount::max(),
        });

    let to: Option<PduCount> = to
        .map(str::parse)
        .transpose()
        .map_err(|_| err!(Request(InvalidParam("Invalid `to` token."))))?;

    let limit = limit.map_or(LIMIT_DEFAULT, |limit| {
        usize_from_ruma_bounded(limit, LIMIT_DEFAULT, LIMIT_MAX)
    });

    let it = match dir {
        Direction::Forward => services
            .rooms
            .timeline
            .pdus(Some(sender_user), room_id, Some(from))
            .ignore_err()
            .left_stream(),

        Direction::Backward => services
            .rooms
            .timeline
            .pdus_rev(Some(sender_user), room_id, Some(from))
            .ignore_err()
            .right_stream(),
    };

    let encrypted = services
        .rooms
        .state_accessor
        .is_encrypted_room(room_id)
        .await;

    let mut scanned = None;
    let reached_to = |count: PduCount| {
        to.is_some_and(|to| match dir {
            Direction::Forward => count >= to,
            Direction::Backward => count <= to,
        })
    };

    let events: Vec<_> = it
        .inspect(|(count, _)| scanned = Some(*count))
        .ready_take_while(|(count, _)| !reached_to(*count))
        .ready_filter_map(|item| event_filter(item, filter))
        .wide_filter_map(|item| event_filters(services, sender_user, item, bypass_visibility))
        .take(limit)
        .wide_then(|item| add_membership_unsigned(services, item, sender_user, encrypted))
        .collect()
        .await;

    let lazy_loading_context = sender_device.map(|device_id| lazy_loading::Context {
        user_id: sender_user,
        device_id,
        room_id,
        token: Some(from.into_unsigned()),
        options: Some(&filter.lazy_load_options),
    });

    let witness = lazy_loading_context
        .as_ref()
        .filter(|_| filter.lazy_load_options.is_enabled())
        .is_some()
        .then_async(|| {
            let ctx = lazy_loading_context
                .as_ref()
                .expect("context present when lazy loading");

            lazy_loading_witness(services, ctx, events.iter())
        });

    let state = witness
        .map(Option::into_iter)
        .map(|option| option.flat_map(Witness::into_iter))
        .map(IterStream::stream)
        .into_stream()
        .flatten()
        .broad_filter_map(async |user_id| get_member_event(services, room_id, &user_id).await)
        .collect()
        .await;

    // `inspect` records the rejected boundary item, distinguishing a `to` stop
    // from stream exhaustion.
    let stopped_at_to = scanned.is_some_and(reached_to);
    let exhausted = matches!(dir, Direction::Backward) && events.len() < limit && !stopped_at_to;
    let next_token = if exhausted {
        scanned
    } else {
        events.last().map(at!(0))
    };

    let chunk = events
        .into_iter()
        .map(at!(1))
        .map(PduEvent::into_room_event)
        .collect();

    let mut response = get_message_events::v3::Response::new();
    response.start = from.to_string();
    response.end = next_token.as_ref().map(ToString::to_string);
    response.chunk = chunk;
    response.state = state;

    Ok(response)
}

pub(crate) async fn lazy_loading_witness<'a, I>(
    services: &Services,
    lazy_loading_context: &lazy_loading::Context<'_>,
    events: I,
) -> Witness
where
    I: Iterator<Item = &'a PdusIterItem> + Clone + Send,
{
    let oldest = events
        .clone()
        .map(|(count, _)| count)
        .copied()
        .min()
        .unwrap_or_else(PduCount::max);

    let newest = events
        .clone()
        .map(|(count, _)| count)
        .copied()
        .max()
        .unwrap_or_else(PduCount::max);

    let receipts = services
        .rooms
        .read_receipt
        .readreceipts_since(lazy_loading_context.room_id, oldest.into_unsigned());

    pin_mut!(receipts);
    let witness: Witness = events
        .stream()
        .map(ref_at!(1))
        .map(Event::sender)
        .map(ToOwned::to_owned)
        .chain(
            receipts
                .ready_take_while(|(_, c, _)| *c <= newest.into_unsigned())
                .map(|(user_id, ..)| user_id.to_owned()),
        )
        .collect()
        .await;

    services
        .rooms
        .lazy_loading
        .witness_retain(witness, lazy_loading_context)
        .await
}

async fn get_member_event(
    services: &Services,
    room_id: &RoomId,
    user_id: &UserId,
) -> Option<Raw<AnyStateEvent>> {
    services
        .rooms
        .state_accessor
        .room_state_get(room_id, &StateEventType::RoomMember, user_id.as_str())
        .map_ok(PduEvent::into_state_event)
        .await
        .ok()
}

pub(crate) async fn event_filters(
    services: &Services,
    user_id: &UserId,
    item: PdusIterItem,
    bypass_visibility: bool,
) -> Option<PdusIterItem> {
    if bypass_visibility {
        return Some(item);
    }

    let item = ignored_filter(services, item, user_id).await?;
    let item = visibility_filter(services, item, user_id).await?;

    Some(item)
}

#[inline]
pub(crate) async fn ignored_filter(
    services: &Services,
    item: PdusIterItem,
    user_id: &UserId,
) -> Option<PdusIterItem> {
    let (_, ref pdu) = item;

    is_ignored_pdu(services, pdu, user_id)
        .await
        .eq(&false)
        .then_some(item)
}

#[inline]
pub(crate) async fn is_ignored_pdu<Pdu>(services: &Services, event: &Pdu, user_id: &UserId) -> bool
where
    Pdu: Event,
{
    // exclude Synapse's dummy events from bloating up response bodies. clients
    // don't need to see this.
    if event.event_type().to_string() == "org.matrix.dummy_event" {
        return true;
    }

    if !IGNORED_MESSAGE_TYPES.contains(event.event_type()) {
        return false;
    }

    let ignored_server = services
        .moderation
        .forbids(event.sender().server_name(), Restriction::Federation);

    ignored_server
        || services
            .users
            .user_is_ignored(event.sender(), user_id)
            .await
}

#[inline]
pub(crate) fn event_filter(item: PdusIterItem, filter: &RoomEventFilter) -> Option<PdusIterItem> {
    let (_, pdu) = &item;
    pdu.matches(filter).then_some(item)
}

/// MSC4115: stamp `unsigned.membership` on a served PDU with the requesting
/// user's membership at the time of the event. The MSC permits omitting the
/// property when calculating it is expensive, so the project restricts it to
/// encrypted rooms where membership-vs-event ordering matters for key share.
#[inline]
pub(crate) async fn annotate_membership(
    services: &Services,
    pdu: &mut PduEvent,
    user_id: &UserId,
    encrypted: bool,
) {
    if !encrypted {
        return;
    }

    let membership = user_membership_at_pdu(services, user_id, pdu).await;

    add_membership(pdu, &membership).log_err().ok();
}

/// `annotate_membership` consume-and-return adapter for stream chains.
#[inline]
pub(crate) async fn with_membership(
    services: &Services,
    mut pdu: PduEvent,
    user_id: &UserId,
    encrypted: bool,
) -> PduEvent {
    annotate_membership(services, &mut pdu, user_id, encrypted).await;
    pdu
}

/// `with_membership` adapter for timeline-iterator items.
#[inline]
pub(crate) async fn add_membership_unsigned(
    services: &Services,
    (count, pdu): PdusIterItem,
    user_id: &UserId,
    encrypted: bool,
) -> PdusIterItem {
    (
        count,
        with_membership(services, pdu, user_id, encrypted).await,
    )
}

/// Whether the user may read the room at all: a current or past member, an
/// invitee, or anyone while the history is world-readable. Per-event
/// visibility is still applied afterwards.
pub(crate) async fn user_can_see_room(
    services: &Services,
    user_id: &UserId,
    room_id: &RoomId,
) -> bool {
    let state_cache = &services.rooms.state_cache;

    state_cache.is_joined(user_id, room_id).await
        || state_cache.is_invited(user_id, room_id).await
        || state_cache.is_left(user_id, room_id).await
        || services
            .rooms
            .state_accessor
            .is_world_readable(room_id)
            .await
}

/// The user's membership as of `pdu`, read from the event itself when it is
/// that user's member event, otherwise from the room state at the event.
async fn user_membership_at_pdu(
    services: &Services,
    user_id: &UserId,
    pdu: &PduEvent,
) -> MembershipState {
    if pdu.kind == TimelineEventType::RoomMember
        && pdu.state_key.as_deref() == Some(user_id.as_str())
        && let Ok(content) = pdu.get_content::<RoomMemberEventContent>()
    {
        return content.membership;
    }

    let state_accessor = &services.rooms.state_accessor;
    let Ok(shortstatehash) = state_accessor.pdu_shortstatehash(&pdu.event_id).await else {
        return MembershipState::Leave;
    };

    state_accessor
        .user_membership(shortstatehash, user_id)
        .await
}

/// Sets `unsigned.membership`, keeping the other unsigned properties.
fn add_membership(pdu: &mut PduEvent, membership: &MembershipState) -> Result {
    let mut unsigned: serde_json::Map<String, serde_json::Value> = pdu
        .unsigned
        .as_deref()
        .map(|raw| serde_json::from_str(raw.get()))
        .transpose()
        .map_err(|e| err!(Database("Invalid unsigned in pdu event: {e}")))?
        .unwrap_or_default();

    unsigned.insert("membership".to_owned(), serde_json::to_value(membership)?);
    pdu.unsigned = Some(to_raw_value(&unsigned)?);

    Ok(())
}
