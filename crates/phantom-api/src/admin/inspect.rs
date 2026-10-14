//! The room inspector: a room's state, timeline, forward extremities and
//! servers, as the server holds them, for debugging a room that has gone
//! wrong — usually over federation.
//!
//! These answer with the events as stored, whoever may see them: an admin
//! debugging a room needs what the server has, not what one member would be
//! shown.

use std::str::FromStr;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use futures::{StreamExt, TryStreamExt};
use phantom_core::{Err, Result, err, matrix::pdu::PduCount};
use phantom_service::rooms::timeline::PdusIterItem;
use ruma::{OwnedEventId, OwnedRoomId, RoomId};
use serde::{Deserialize, Serialize};
use serde_json::{json, value::RawValue};

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct StateEvent {
    event_id: OwnedEventId,
    #[serde(rename = "type")]
    kind: String,
    state_key: String,
    sender: String,
    origin_server_ts: u64,
    content: Box<RawValue>,
}

/// # `GET /_phantom/admin/v1/rooms/{room_id}/state`
///
/// The room's current state, one event per type and state key, sorted by
/// both.
pub(super) async fn state(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    known(&services, &room_id).await?;

    let mut events: Vec<StateEvent> = services
        .rooms
        .state_accessor
        .room_state_full_pdus(&room_id)
        .map_ok(|pdu| StateEvent {
            kind: pdu.kind.to_string(),
            state_key: pdu.state_key.as_deref().unwrap_or_default().to_owned(),
            sender: pdu.sender.to_string(),
            origin_server_ts: pdu.origin_server_ts.into(),
            content: pdu.content,
            event_id: pdu.event_id,
        })
        .try_collect()
        .await?;

    events.sort_by(|a, b| (&a.kind, &a.state_key).cmp(&(&b.kind, &b.state_key)));

    Ok(Json(events))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct EventsQuery {
    limit: Option<usize>,

    /// The `next` of the page before: events older than it.
    from: Option<String>,
}

#[derive(Serialize)]
pub(super) struct TimelineEvent {
    event_id: OwnedEventId,
    #[serde(rename = "type")]
    kind: String,
    state_key: Option<String>,
    sender: String,
    origin_server_ts: u64,
    depth: u64,
    prev_events: Vec<OwnedEventId>,
    content: Box<RawValue>,
}

/// # `GET /_phantom/admin/v1/rooms/{room_id}/events?limit=&from=`
///
/// The room's timeline, newest first, with each event's place in the graph:
/// its depth and the events it follows. `next`, when set, asks for the page
/// of older events after this one.
pub(super) async fn events(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse> {
    known(&services, &room_id).await?;

    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let from = query
        .from
        .as_deref()
        .map(PduCount::from_str)
        .transpose()
        .map_err(|e| {
            err!(Request(InvalidParam(
                "from is not a position in the timeline: {e}"
            )))
        })?;

    let page: Vec<PdusIterItem> = services
        .rooms
        .timeline
        .pdus_rev(None, &room_id, from)
        .try_filter(|(count, _)| std::future::ready(from.is_none_or(|from| *count < from)))
        .take(limit)
        .try_collect()
        .await?;

    let next = page
        .last()
        .filter(|_| page.len() == limit)
        .map(|(count, _)| count.to_string());

    let events: Vec<TimelineEvent> = page
        .into_iter()
        .map(|(_, pdu)| TimelineEvent {
            kind: pdu.kind.to_string(),
            state_key: pdu.state_key.as_deref().map(ToOwned::to_owned),
            sender: pdu.sender.to_string(),
            origin_server_ts: pdu.origin_server_ts.into(),
            depth: pdu.depth.into(),
            prev_events: pdu.prev_events,
            content: pdu.content,
            event_id: pdu.event_id,
        })
        .collect();

    Ok(Json(json!({ "events": events, "next": next })))
}

#[derive(Serialize)]
pub(super) struct Extremity {
    event_id: OwnedEventId,

    /// None for an extremity the server lists but does not hold, which is
    /// itself worth knowing.
    depth: Option<u64>,
    sender: Option<String>,
    origin_server_ts: Option<u64>,
}

/// # `GET /_phantom/admin/v1/rooms/{room_id}/extremities`
///
/// The room's forward extremities: the newest events the server knows,
/// which the next event it sends will follow. More than a handful usually
/// means the room's graph has forked and not been merged again.
pub(super) async fn extremities(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    known(&services, &room_id).await?;

    let ids: Vec<OwnedEventId> = services
        .rooms
        .state
        .get_forward_extremities(&room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let mut extremities = Vec::with_capacity(ids.len());
    for event_id in ids {
        let pdu = services.rooms.timeline.get_pdu(&event_id).await.ok();

        extremities.push(Extremity {
            depth: pdu.as_ref().map(|pdu| pdu.depth.into()),
            sender: pdu.as_ref().map(|pdu| pdu.sender.to_string()),
            origin_server_ts: pdu.as_ref().map(|pdu| pdu.origin_server_ts.into()),
            event_id,
        });
    }

    extremities.sort_by_key(|extremity| std::cmp::Reverse(extremity.depth));

    Ok(Json(extremities))
}

/// # `GET /_phantom/admin/v1/rooms/{room_id}/servers`
///
/// The servers with a member joined to the room, which are the ones its
/// events are sent to.
pub(super) async fn servers(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    known(&services, &room_id).await?;

    let mut servers: Vec<String> = services
        .rooms
        .state_cache
        .room_servers(&room_id)
        .map(ToString::to_string)
        .collect()
        .await;

    servers.sort();

    Ok(Json(servers))
}

async fn known(services: &RouterState, room_id: &RoomId) -> Result {
    if !services.rooms.metadata.exists(room_id).await {
        return Err!(Request(NotFound("The server does not know {room_id}.")));
    }

    Ok(())
}
