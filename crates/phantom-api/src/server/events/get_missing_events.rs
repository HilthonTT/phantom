use std::collections::{BTreeSet, VecDeque};

use axum::extract::State;
use futures::{StreamExt, TryStreamExt, future::try_join, stream::FuturesOrdered};
use phantom_core::{Result, debug, err, math::usize_from_ruma_bounded, stream::automatic_width};
use phantom_service::Services;
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, EventId, OwnedEventId, RoomId, RoomVersionId,
    ServerName, api::federation::event::get_missing_events, canonical_json::redact_in_place,
    room_version_rules::RedactionRules,
};
use serde_json::value::RawValue as RawJsonValue;

use crate::{router::Ruma, server::AccessCheck};

type Seen = BTreeSet<OwnedEventId>;
type Pending = VecDeque<(OwnedEventId, bool)>;

const LIMIT_MAX: usize = 50;

const LIMIT_DEFAULT: usize = 10;

const WALK_MAX: usize = 256;

const EARLIEST_MAX: usize = 4096;

pub(crate) async fn get_missing_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_missing_events::v1::Request>,
) -> Result<get_missing_events::v1::Response> {
    let access_check = AccessCheck {
        services: &services,
        origin: body.origin(),
        room_id: &body.room_id,
        event_id: None,
    };

    let room_version = services.rooms.state.get_room_version(&body.room_id);
    let (room_version, ()) = try_join(room_version, access_check.check()).await?;

    let rules = room_version.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}."
        )))
    })?;

    let fetch = async |(event_id, is_latest): (OwnedEventId, bool)| {
        let event = services.rooms.timeline.get_pdu_json(&event_id).await;

        (event_id, is_latest, event)
    };

    let limit = usize_from_ruma_bounded(body.limit, LIMIT_DEFAULT, LIMIT_MAX);

    let (seen, pending) = walk_seed(&body);
    let seen_max = seen.len().saturating_add(WALK_MAX);

    let mut fetches = FuturesOrdered::new();
    let (mut seen, mut pending) = (seen, pending);

    let mut walked = Vec::with_capacity(limit);
    while walked.len() < limit {
        let Some(event) = next_missing_event(
            &body,
            &fetch,
            seen_max,
            &mut seen,
            &mut pending,
            &mut fetches,
        )
        .await
        else {
            break;
        };

        walked.push(event);
    }

    let mut formatted = FuturesOrdered::new();
    for (event_id, event) in walked {
        formatted.push_back(format_missing_event(
            &services,
            body.origin(),
            &body.room_id,
            &room_version,
            &rules.redaction,
            event_id,
            event,
        ));
    }

    let mut events: Vec<_> = formatted.try_collect().await?;
    events.reverse();

    Ok(get_missing_events::v1::Response::new(events))
}

async fn format_missing_event(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
    room_version: &RoomVersionId,
    redaction_rules: &RedactionRules,
    event_id: OwnedEventId,
    mut event: CanonicalJsonObject,
) -> Result<Box<RawJsonValue>> {
    let visible = services
        .rooms
        .state_accessor
        .server_can_see_event(origin, room_id, &event_id)
        .await;

    if !visible {
        redact_in_place(&mut event, redaction_rules, None)
            .map_err(|error| err!(Database("Failed to redact event: {error}")))?;
    }

    Ok(services
        .federation
        .format_pdu(event, Some(room_version))
        .await)
}

fn walk_seed(body: &get_missing_events::v1::Request) -> (Seen, Pending) {
    let mut seen: Seen = body
        .earliest_events
        .iter()
        .take(EARLIEST_MAX)
        .cloned()
        .collect();

    let pending = body
        .latest_events
        .iter()
        .take(WALK_MAX)
        .filter(|event_id| seen.insert((*event_id).clone()))
        .cloned()
        .map(|event_id| (event_id, true))
        .collect();

    (seen, pending)
}

async fn next_missing_event<Fetch, Fut>(
    body: &Ruma<get_missing_events::v1::Request>,
    fetch: &Fetch,
    seen_max: usize,
    seen: &mut Seen,
    pending: &mut Pending,
    fetches: &mut FuturesOrdered<Fut>,
) -> Option<(OwnedEventId, CanonicalJsonObject)>
where
    Fetch: Fn((OwnedEventId, bool)) -> Fut + Sync,
    Fut: Future<Output = (OwnedEventId, bool, Result<CanonicalJsonObject>)> + Send,
{
    loop {
        let width = automatic_width();

        while fetches.len() < width
            && let Some(input) = pending.pop_front()
        {
            fetches.push_back(fetch(input));
        }

        let (event_id, is_latest, event) = fetches.next().await?;
        let Ok(event) = event else {
            debug!(
                ?body.origin,
                %event_id,
                "Event does not exist locally, skipping"
            );

            continue;
        };

        if event.get("room_id").and_then(CanonicalJsonValue::as_str) != Some(body.room_id.as_str())
        {
            continue;
        }

        event
            .get("prev_events")
            .and_then(CanonicalJsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(CanonicalJsonValue::as_str)
            .filter_map(|event_id| EventId::parse(event_id).ok())
            .filter(|event_id| seen.len() < seen_max && seen.insert(event_id.clone()))
            .for_each(|event_id| pending.push_back((event_id, false)));

        if !is_latest {
            return Some((event_id, event));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    use axum_extra::extract::cookie::CookieJar;
    use futures::stream::FuturesOrdered;
    use ruma::{
        CanonicalJsonObject, EventId, OwnedEventId, RoomId,
        api::federation::event::get_missing_events::v1::Request, room_id, server_name,
    };
    use serde_json::json;

    use super::{EARLIEST_MAX, Ruma, WALK_MAX, err, next_missing_event, walk_seed};

    const MAX_PREV_EVENTS: usize = 20;

    fn event_ids(prefix: &str, len: usize) -> Vec<OwnedEventId> {
        (0..len)
            .map(|index| {
                EventId::parse(format!("${prefix}{index}:example.com")).expect("valid event id")
            })
            .collect()
    }

    fn request(earliest: Vec<OwnedEventId>, latest: Vec<OwnedEventId>) -> Ruma<Request> {
        let body = Request::new(room_id!("!room:example.com").to_owned(), earliest, latest);

        Ruma {
            body,
            cookies: CookieJar::new(),
            origin: Some(server_name!("example.com").to_owned()),
            sender_user: None,
            sender_device: None,
            appservice_info: None,
            json_body: None,
        }
    }

    fn event_with_prevs(room_id: &RoomId, index: usize) -> CanonicalJsonObject {
        let prev_events: Vec<_> = (0..MAX_PREV_EVENTS)
            .map(|prev| format!("$p{index}a{prev}:example.com"))
            .collect();
        let value = json!({
            "room_id": room_id,
            "prev_events": prev_events,
        });

        serde_json::from_value(value).expect("valid canonical json")
    }

    #[test]
    fn seed_bounded_by_cap_not_request() {
        let body = request(Vec::new(), event_ids("latest", 5_000));
        let (seen, pending) = walk_seed(&body);

        assert_eq!(seen.len(), WALK_MAX);
        assert_eq!(pending.len(), WALK_MAX);

        let body = request(event_ids("earliest", 10_000), event_ids("latest", 5_000));
        let (seen, pending) = walk_seed(&body);

        assert_eq!(seen.len(), EARLIEST_MAX + WALK_MAX);
        assert_eq!(pending.len(), WALK_MAX);
    }

    #[tokio::test]
    async fn walk_lookups_bounded_by_cap_not_request() {
        let body = request(Vec::new(), event_ids("latest", 5_000));
        let (mut seen, mut pending) = walk_seed(&body);
        let seen_max = seen.len().saturating_add(WALK_MAX);
        let lookups = AtomicUsize::new(0);
        let room_id = body.room_id.clone();
        let fetch = async |(event_id, is_latest): (OwnedEventId, bool)| {
            let index = lookups.fetch_add(1, Relaxed);
            let event = is_latest
                .then(|| event_with_prevs(&room_id, index))
                .ok_or_else(|| err!(Request(NotFound("Event not found."))));

            (event_id, is_latest, event)
        };

        let mut fetches = FuturesOrdered::new();

        let result = next_missing_event(
            &body,
            &fetch,
            seen_max,
            &mut seen,
            &mut pending,
            &mut fetches,
        )
        .await;

        assert!(result.is_none());
        assert_eq!(lookups.load(Relaxed), 2 * WALK_MAX);
    }
}
