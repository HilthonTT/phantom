use ruma::{OwnedUserId, UserId};
use serde::Deserialize;

use super::*;

/// Events are sorted from "earliest" to "latest".
///
/// They are compared using the negative power level (reverse topological
/// ordering), the origin server timestamp and in case of a tie the `EventId`s
/// are compared lexicographically.
///
/// The power level is negative because a higher power level is equated to an
/// earlier (further back in time) origin server timestamp.
#[tracing::instrument(level = "debug", skip_all)]
pub(super) async fn reverse_topological_power_sort<E, F, Fut>(
    room_version: &RoomVersion,
    events_to_sort: Vec<E::Id>,
    auth_diff: &HashSet<E::Id>,
    fetch_event: &F,
    parallel_fetches: usize,
) -> Result<Vec<E::Id>>
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Send + Sync,
    E::Id: Borrow<EventId> + Send + Sync,
{
    debug!("reverse topological sort of power events");

    let mut graph = HashMap::new();
    for event_id in events_to_sort {
        add_event_and_auth_chain_to_graph(&mut graph, event_id, auth_diff, fetch_event).await;
    }

    let event_to_pl = stream::iter(graph.keys())
        .map(|event_id| {
            get_power_level_for_sender(
                room_version,
                event_id.clone(),
                fetch_event,
                parallel_fetches,
            )
            .map(move |res| res.map(|pl| (event_id, pl)))
        })
        .buffer_unordered(parallel_fetches)
        .try_fold(HashMap::new(), |mut event_to_pl, (event_id, pl)| {
            debug!(
                event_id = event_id.borrow().as_str(),
                power_level = i64::from(pl),
                "found the power level of an event's sender",
            );

            event_to_pl.insert(event_id.clone(), pl);
            future::ok(event_to_pl)
        })
        .boxed()
        .await?;

    let event_to_pl = &event_to_pl;
    let fetcher = |event_id: E::Id| async move {
        let pl = *event_to_pl
            .get(event_id.borrow())
            .ok_or_else(|| Error::NotFound(String::new()))?;
        let ev = fetch_event(event_id)
            .await
            .ok_or_else(|| Error::NotFound(String::new()))?;
        Ok((pl, ev.origin_server_ts()))
    };

    lexicographical_topological_sort(&graph, &fetcher).await
}

/// Sorts the event graph based on number of outgoing/incoming edges.
///
/// `key_fn` is used as to obtain the power level and age of an event for
/// breaking ties (together with the event ID).
#[tracing::instrument(level = "debug", skip_all)]
pub async fn lexicographical_topological_sort<Id, F, Fut, Hasher>(
    graph: &HashMap<Id, HashSet<Id, Hasher>>,
    key_fn: &F,
) -> Result<Vec<Id>>
where
    F: Fn(Id) -> Fut + Sync,
    Fut: Future<Output = Result<(Int, MilliSecondsSinceUnixEpoch)>> + Send,
    Id: Borrow<EventId> + Clone + Eq + Hash + Ord + Send + Sync,
    Hasher: BuildHasher + Default + Clone + Send + Sync,
{
    #[derive(PartialEq, Eq)]
    struct TieBreaker<'a, Id> {
        power_level: Int,
        origin_server_ts: MilliSecondsSinceUnixEpoch,
        event_id: &'a Id,
    }

    impl<Id> Ord for TieBreaker<'_, Id>
    where
        Id: Ord,
    {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .power_level
                .cmp(&self.power_level)
                .then(self.origin_server_ts.cmp(&other.origin_server_ts))
                .then(self.event_id.cmp(other.event_id))
        }
    }

    impl<Id> PartialOrd for TieBreaker<'_, Id>
    where
        Id: Ord,
    {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    debug!("starting lexicographical topological sort");

    let mut outdegree_map = graph.clone();

    let mut reverse_graph: HashMap<_, HashSet<_, Hasher>> = HashMap::new();

    let mut zero_outdegree = Vec::new();

    for (node, edges) in graph {
        if edges.is_empty() {
            let (power_level, origin_server_ts) = key_fn(node.clone()).await?;
            zero_outdegree.push(Reverse(TieBreaker {
                power_level,
                origin_server_ts,
                event_id: node,
            }));
        }

        reverse_graph.entry(node).or_default();
        for edge in edges {
            reverse_graph.entry(edge).or_default().insert(node);
        }
    }

    let mut heap = BinaryHeap::from(zero_outdegree);

    let mut sorted = vec![];
    while let Some(Reverse(item)) = heap.pop() {
        let node = item.event_id;

        for &parent in reverse_graph
            .get(node)
            .expect("EventId in heap is also in reverse_graph")
        {
            let out = outdegree_map
                .get_mut(parent.borrow())
                .expect("outdegree_map knows of all referenced EventIds");

            out.remove(node.borrow());
            if out.is_empty() {
                let (power_level, origin_server_ts) = key_fn(parent.clone()).await?;
                heap.push(Reverse(TieBreaker {
                    power_level,
                    origin_server_ts,
                    event_id: parent,
                }));
            }
        }

        sorted.push(node.clone());
    }

    Ok(sorted)
}

/// Find the power level for the sender of `event_id` or return a default value
/// of zero.
///
/// Do NOT use this any where but topological sort, we find the power level for
/// the eventId at the eventId's generation (we walk backwards to `EventId`s
/// most recent previous power level event).
async fn get_power_level_for_sender<E, F, Fut>(
    room_version: &RoomVersion,
    event_id: E::Id,
    fetch_event: &F,
    parallel_fetches: usize,
) -> serde_json::Result<Int>
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Send,
    E::Id: Borrow<EventId> + Send,
{
    debug!("fetch event ({event_id}) senders power level");

    let event = fetch_event(event_id.clone()).await;

    let auth_events = event.as_ref().map(Event::auth_events).into_iter().flatten();

    let auth_events = stream::iter(auth_events)
        .map(|aid| fetch_event(aid.clone()))
        .buffer_unordered(parallel_fetches.min(5))
        .filter_map(future::ready)
        .collect::<Vec<_>>()
        .boxed()
        .await;

    let pl = auth_events
        .iter()
        .find(|aev| is_type_and_key(aev, &TimelineEventType::RoomPowerLevels, ""));

    let content: PowerLevelsContentFields = match pl {
        None => {
            // Diverges from upstream, which gave every sender level 0 here.
            // Without an m.room.power_levels event the room's creator has
            // level 100 (this is what the auth rules and Synapse do), and
            // sorting the creator's events as level 0 resolved early
            // conflicts differently from other servers.
            let is_creator = auth_events
                .iter()
                .find(|aev| is_type_and_key(aev, &TimelineEventType::RoomCreate, ""))
                .is_some_and(|create| {
                    event
                        .as_ref()
                        .is_some_and(|event| is_room_creator(room_version, create, event.sender()))
                });

            return Ok(if is_creator { int!(100) } else { int!(0) });
        }
        Some(ev) => from_json_str(ev.content().get())?,
    };

    if let Some(ev) = event
        && let Some(&user_level) = content.get_user_power(ev.sender())
    {
        debug!("found {} at power_level {user_level}", ev.sender());
        return Ok(user_level);
    }

    Ok(content.users_default)
}

/// Whether `sender` created the room, going by its `m.room.create` event.
fn is_room_creator(room_version: &RoomVersion, create: &impl Event, sender: &UserId) -> bool {
    if room_version.use_room_create_sender {
        return create.sender() == sender;
    }

    #[derive(Deserialize)]
    struct RoomCreateContentFields {
        creator: Option<OwnedUserId>,
    }

    from_json_str::<RoomCreateContentFields>(create.content().get())
        .ok()
        .and_then(|content| content.creator)
        .is_some_and(|creator| creator == sender)
}

async fn add_event_and_auth_chain_to_graph<E, F, Fut>(
    graph: &mut HashMap<E::Id, HashSet<E::Id>>,
    event_id: E::Id,
    auth_diff: &HashSet<E::Id>,
    fetch_event: &F,
) where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Send + Sync,
    E::Id: Borrow<EventId> + Clone + Send + Sync,
{
    let mut state = vec![event_id];
    while let Some(eid) = state.pop() {
        graph.entry(eid.clone()).or_default();
        let event = fetch_event(eid.clone()).await;
        let auth_events = event.as_ref().map(Event::auth_events).into_iter().flatten();

        for aid in auth_events {
            if auth_diff.contains(aid.borrow()) {
                if !graph.contains_key(aid.borrow()) {
                    state.push(aid.to_owned());
                }

                graph.get_mut(eid.borrow()).unwrap().insert(aid.to_owned());
            }
        }
    }
}

pub(super) async fn is_power_event_id<E, F, Fut>(event_id: &E::Id, fetch: &F) -> bool
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Send,
    E::Id: Borrow<EventId> + Send + Sync,
{
    match fetch(event_id.clone()).await.as_ref() {
        Some(state) => is_power_event(state),
        _ => false,
    }
}
