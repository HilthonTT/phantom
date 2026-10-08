use axum::extract::State;
use futures::{FutureExt, Stream, StreamExt, future::pending, pin_mut};
use phantom_core::{
    Err, Result, err,
    matrix::{PduCount, PduEvent},
    result::FlatOk,
    stream::{ReadyExt, WidebandExt},
};
use phantom_service::{Services, rooms::timeline::PdusIterItem};
use ruma::{
    UserId,
    api::client::peeking::listen_to_new_events::v3::{Request, Response},
    events::{
        StateEventType,
        room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
    },
};
use tokio::time::{Duration, Instant, timeout_at};

use crate::{client::visibility_filter, router::Ruma};

const EVENT_LIMIT: usize = 50;

/// One user's listen on one room's event stream.
///
/// A non-member's listen is a peek, which sees only what a room preview may
/// show.
struct Listen<'a> {
    services: &'a Services,
    sender_user: &'a UserId,
    peeking: bool,
}

/// GET `/_matrix/client/v3/events`
pub(crate) async fn events_route(
    State(services): State<crate::router::State>,
    body: Ruma<Request>,
) -> Result<Response> {
    let sender_user = body.sender_user();

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

    let room_id = body.room_id.as_ref();

    let peeking = !services
        .rooms
        .state_cache
        .is_joined(sender_user, room_id)
        .await;

    if peeking
        && !services
            .rooms
            .state_accessor
            .is_world_readable(room_id)
            .await
    {
        return Err!(Request(Forbidden("No room preview available.")));
    }

    // The endpoint listens for new events, so a stream without a token starts now.
    let from = body
        .body
        .from
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|_| err!(Request(InvalidParam("Invalid `from` token."))))?
        .unwrap_or_else(|| PduCount::Normal(services.server_state.current_count()));

    let listen = Listen {
        services: &services,
        sender_user,
        peeking,
    };

    let stop_at = Instant::now()
        .checked_add(Duration::from_millis(timeout))
        .expect("configuration must limit maximum timeout");

    loop {
        // Without a device there is nothing to watch, so the listen runs out its
        // timeout.
        let watchers = match body.sender_device.as_deref() {
            Some(device_id) => services.sync.watch(sender_user, device_id).boxed(),
            None => pending().boxed(),
        };

        let next_batch = services.server_state.current_count();

        let window = services
            .rooms
            .timeline
            .pdus(Some(sender_user), room_id, Some(from))
            .ready_filter_map(Result::ok)
            .ready_take_while(|(count, _)| PduCount::Normal(next_batch).ge(count));

        // Any new event answers, hidden or not, so no later wake rescans the window.
        if let Some(response) = window_page(&listen, window, from, next_batch).await {
            return Ok(response);
        }

        if timeout_at(stop_at, watchers).await.is_err() || services.server.is_stopping() {
            let mut response = Response::new();
            response.start = from.to_string().into();
            response.end = (!services.server.is_stopping())
                .then_some(next_batch)
                .as_ref()
                .map(ToString::to_string);

            return Ok(response);
        }
    }
}

/// The page for a window of new events, or `None` while the window is empty.
///
/// The peek keeps the event it looked at, so the page scans the window once.
async fn window_page<Window>(
    listen: &Listen<'_>,
    window: Window,
    from: PduCount,
    next_batch: u64,
) -> Option<Response>
where
    Window: Stream<Item = PdusIterItem> + Send,
{
    let window = window.peekable();

    pin_mut!(window);
    window.as_mut().peek().await?;

    visible_page(listen, window, from, next_batch).await.into()
}

/// The events of a window the user may see, as one page of the stream.
///
/// A full page ends at its last event. A short one scanned the whole window,
/// hidden events included, so it ends at `next_batch`. An empty page starts
/// where the stream did.
async fn visible_page<Window>(
    listen: &Listen<'_>,
    window: Window,
    from: PduCount,
    next_batch: u64,
) -> Response
where
    Window: Stream<Item = PdusIterItem> + Send,
{
    let (first, last, chunk) = window
        .wide_filter_map(|item| listen_filter(listen, item))
        .take(EVENT_LIMIT)
        .ready_fold(
            (None, None, Vec::new()),
            |(first, _, mut chunk), (count, pdu)| {
                chunk.push(PduEvent::into_room_event(pdu));
                (first.or(Some(count)), Some(count), chunk)
            },
        )
        .await;

    let start = first.unwrap_or(from).to_string().into();

    let end = last
        .filter(|_| chunk.len().eq(&EVENT_LIMIT))
        .unwrap_or(PduCount::Normal(next_batch))
        .to_string()
        .into();

    let mut response = Response::new();
    response.start = start;
    response.end = end;
    response.chunk = chunk;

    response
}

/// Keeps an event the listener may see.
///
/// A member gets the general history-visibility rule. A peek gets only what a
/// room preview may show: events sent while the room was world-readable, and the
/// event that made it so.
async fn listen_filter(listen: &Listen<'_>, item: PdusIterItem) -> Option<PdusIterItem> {
    if !listen.peeking {
        return visibility_filter(listen.services, item, listen.sender_user).await;
    }

    let (_, pdu) = &item;

    is_world_readable_at(listen.services, pdu)
        .await
        .then_some(item)
}

/// Whether the room's history was world-readable at `pdu`, or `pdu` is the
/// history-visibility event that made it so.
async fn is_world_readable_at(services: &Services, pdu: &PduEvent) -> bool {
    let world_readable = |content: RoomHistoryVisibilityEventContent| {
        content.history_visibility == HistoryVisibility::WorldReadable
    };

    if pdu.kind == StateEventType::RoomHistoryVisibility.into()
        && pdu.state_key.as_deref() == Some("")
        && pdu.get_content().is_ok_and(world_readable)
    {
        return true;
    }

    let state_accessor = &services.rooms.state_accessor;
    let Ok(shortstatehash) = state_accessor.pdu_shortstatehash(&pdu.event_id).await else {
        return false;
    };

    state_accessor
        .state_get_content(shortstatehash, &StateEventType::RoomHistoryVisibility, "")
        .await
        .is_ok_and(world_readable)
}
