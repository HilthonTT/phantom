use axum::extract::State;
use futures::{FutureExt, StreamExt, future::try_join3};
use phantom_core::{
    Err, Result, at,
    bool::BoolExt,
    err,
    math::usize_from_ruma_bounded,
    matrix::pdu::{PduCount, PduEvent},
    result::FlatOk,
    stream::{IterStream, ReadyExt, WidebandExt},
};
use phantom_service::Services;
use ruma::{
    EventId, RoomId, UInt, UserId,
    api::{
        Direction,
        client::relations::{
            get_relating_events, get_relating_events_with_rel_type,
            get_relating_events_with_rel_type_and_event_type,
        },
    },
    events::{TimelineEventType, relation::RelationType},
};

use crate::{
    client::{is_ignored_pdu, utils::sender_ignored},
    router::Ruma,
};

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}/{relType}/{eventType}`
pub(crate) async fn get_relating_events_with_rel_type_and_event_type_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_relating_events_with_rel_type_and_event_type::v1::Request>,
) -> Result<get_relating_events_with_rel_type_and_event_type::v1::Response> {
    paginate_relations_with_filter(
        &services,
        body.sender_user(),
        &body.room_id,
        &body.event_id,
        body.event_type.clone().into(),
        body.rel_type.clone().into(),
        body.from.as_deref(),
        body.to.as_deref(),
        body.limit,
        body.recurse,
        body.dir,
    )
    .await
    .map(|res| {
        let mut response =
            get_relating_events_with_rel_type_and_event_type::v1::Response::new(res.chunk);
        response.next_batch = res.next_batch;
        response.prev_batch = res.prev_batch;
        response.recursion_depth = res.recursion_depth;
        response
    })
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}/{relType}`
pub(crate) async fn get_relating_events_with_rel_type_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_relating_events_with_rel_type::v1::Request>,
) -> Result<get_relating_events_with_rel_type::v1::Response> {
    paginate_relations_with_filter(
        &services,
        body.sender_user(),
        &body.room_id,
        &body.event_id,
        None,
        body.rel_type.clone().into(),
        body.from.as_deref(),
        body.to.as_deref(),
        body.limit,
        body.recurse,
        body.dir,
    )
    .await
    .map(|res| {
        let mut response = get_relating_events_with_rel_type::v1::Response::new(res.chunk);
        response.next_batch = res.next_batch;
        response.prev_batch = res.prev_batch;
        response.recursion_depth = res.recursion_depth;
        response
    })
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/relations/{eventId}`
pub(crate) async fn get_relating_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_relating_events::v1::Request>,
) -> Result<get_relating_events::v1::Response> {
    paginate_relations_with_filter(
        &services,
        body.sender_user(),
        &body.room_id,
        &body.event_id,
        None,
        None,
        body.from.as_deref(),
        body.to.as_deref(),
        body.limit,
        body.recurse,
        body.dir,
    )
    .await
}

#[expect(clippy::too_many_arguments)]
#[tracing::instrument(
    name = "relations",
    level = "debug",
    skip_all,
    fields(room_id, target, from, to, dir, limit, recurse)
)]
async fn paginate_relations_with_filter(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    target: &EventId,
    filter_event_type: Option<TimelineEventType>,
    filter_rel_type: Option<RelationType>,
    from: Option<&str>,
    to: Option<&str>,
    limit: Option<UInt>,
    recurse: bool,
    dir: Direction,
) -> Result<get_relating_events::v1::Response> {
    let from: Option<PduCount> = from.map(str::parse).transpose()?;

    let to: Option<PduCount> = to.map(str::parse).flat_ok();

    // Spec (v1.10) recommends depth of at least 3
    let max_depth: u8 = if recurse { 3 } else { 0 };

    let limit = limit.map_or(30, |limit| usize_from_ruma_bounded(limit, 30, 100));

    let target_count = services.rooms.timeline.get_pdu_count(target).map(Ok);

    let visible = services
        .rooms
        .state_accessor
        .user_can_see_state_events(sender_user, room_id)
        .map(|visible| {
            visible
                .into_option()
                .ok_or_else(|| err!(Request(Forbidden("You cannot view this room."))))
        });

    let target_pdu = services.rooms.timeline.get_pdu(target).map(Ok);

    let (target_count, (), target_pdu) = try_join3(target_count, visible, target_pdu).await?;

    let (Ok(target_count), Ok(target_pdu)) = (target_count, target_pdu) else {
        return Ok(get_relating_events::v1::Response::new(Vec::new()));
    };

    if target_pdu.room_id != room_id {
        return Err!(Request(NotFound("Event not found in room.")));
    }

    if let PduCount::Backfilled(_) = target_count {
        return Ok(get_relating_events::v1::Response::new(Vec::new()));
    }

    if is_ignored_pdu(services, &target_pdu, sender_user).await {
        return Err(sender_ignored(&target_pdu.sender));
    }

    let start = from.unwrap_or_else(|| match dir {
        Direction::Forward => PduCount::min(),
        Direction::Backward => PduCount::max(),
    });

    let relations = services
        .rooms
        .pdu_metadata
        .get_relations(sender_user, room_id, target, start, limit, max_depth, dir)
        .await;

    let events: Vec<_> = relations
        .into_iter()
        .stream()
        .ready_filter(|(count, _)| matches!(count, PduCount::Normal(_)))
        .ready_take_while(|&(count, _)| Some(count) != to)
        .ready_filter(|(_, pdu)| {
            filter_event_type
                .as_ref()
                .is_none_or(|kind| *kind == pdu.kind)
        })
        .ready_filter(|(_, pdu)| {
            filter_rel_type
                .as_ref()
                .is_none_or(|rel_type| pdu.relation_type_equal(rel_type))
        })
        .wide_filter_map(async |(count, pdu)| {
            services
                .rooms
                .state_accessor
                .user_can_see_event(sender_user, room_id, &pdu.event_id)
                .await
                .then_some((count, pdu))
        })
        .take(limit)
        .collect()
        .await;

    let mut response = get_relating_events::v1::Response::new(Vec::new());
    response.recursion_depth = recurse.then(|| max_depth.into());

    response.next_batch = events.last().map(at!(0)).as_ref().map(ToString::to_string);

    response.prev_batch = events
        .first()
        .map(at!(0))
        .or(from)
        .as_ref()
        .map(ToString::to_string);

    response.chunk = events
        .into_iter()
        .map(at!(1))
        .map(PduEvent::into_message_like_event)
        .collect();

    Ok(response)
}
