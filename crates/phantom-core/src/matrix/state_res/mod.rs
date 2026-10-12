#![cfg_attr(test, allow(warnings))]

mod auth_check;
pub mod error;
pub mod event_auth;
mod event_type_ext;
mod mainline;
mod power_levels;
mod room_version;
mod sort;

#[cfg(test)]
mod fixtures;

#[cfg(all(test, phantom_bench))]
mod benches;

use std::{
    borrow::Borrow,
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap, HashSet},
    fmt::Debug,
    hash::{BuildHasher, Hash},
};

use futures::{Future, FutureExt, StreamExt, TryFutureExt, TryStreamExt, future, stream};
use ruma::{
    EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, RoomId, RoomVersionId,
    events::{
        StateEventType, TimelineEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
    int,
};
use serde_json::from_str as from_json_str;

use self::{auth_check::iterative_auth_check, mainline::mainline_sort};
pub use self::{
    error::Error,
    event_auth::{auth_check, auth_types_for_event},
    event_type_ext::EventTypeExt,
    room_version::RoomVersion,
    sort::lexicographical_topological_sort,
};
use self::{
    power_levels::PowerLevelsContentFields,
    sort::{is_power_event_id, reverse_topological_power_sort},
};
use crate::{
    debug,
    matrix::{event::Event, pdu::StateKey},
    trace, warn,
};

pub type StateMap<T> = HashMap<TypeStateKey, T>;
pub type StateMapItem<T> = (TypeStateKey, T);
pub type TypeStateKey = (StateEventType, StateKey);

type Result<T, E = Error> = crate::Result<T, E>;

pub async fn resolve<'a, E, Sets, SetIter, Hasher, Fetch, FetchFut, Exists, ExistsFut>(
    room_version: &RoomVersionId,
    state_sets: Sets,
    auth_chain_sets: &'a [HashSet<E::Id, Hasher>],
    event_fetch: &Fetch,
    event_exists: &Exists,
    parallel_fetches: usize,
) -> Result<StateMap<E::Id>>
where
    Fetch: Fn(E::Id) -> FetchFut + Sync,
    FetchFut: Future<Output = Option<E>> + Send,
    Exists: Fn(E::Id) -> ExistsFut + Sync,
    ExistsFut: Future<Output = bool> + Send,
    Sets: IntoIterator<IntoIter = SetIter> + Send,
    SetIter: Iterator<Item = &'a StateMap<E::Id>> + Clone + Send,
    Hasher: BuildHasher + Send + Sync,
    E: Event + Clone + Send + Sync,
    E::Id: Borrow<EventId> + From<OwnedEventId> + Send + Sync,
    for<'b> &'b E: Send,
{
    debug!("State resolution starting");

    let room_version = RoomVersion::new(room_version)?;

    let (clean, conflicting) = separate(state_sets.into_iter());

    debug!(count = clean.len(), "non-conflicting events");
    trace!(map = ?clean, "non-conflicting events");

    if conflicting.is_empty() {
        debug!("no conflicting state found");
        return Ok(clean);
    }

    debug!(count = conflicting.len(), "conflicting events");
    trace!(map = ?conflicting, "conflicting events");

    let conflicted_set: HashSet<E::Id> = conflicting.into_values().flatten().collect();

    // State resolution v2.1 also counts every event on an auth path between
    // two conflicted events.
    let subgraph = if room_version.state_res_v2_1 {
        conflicted_state_subgraph(&conflicted_set, event_fetch).await
    } else {
        HashSet::new()
    };

    debug!(count = subgraph.len(), "conflicted state subgraph");

    let auth_chain_diff = get_auth_chain_diff(auth_chain_sets)
        .chain(conflicted_set)
        .chain(subgraph);

    let all_conflicted: HashSet<_> = stream::iter(auth_chain_diff)
        .map(|id| event_exists(id.clone()).map(move |exists| (id, exists)))
        .buffer_unordered(parallel_fetches)
        .filter_map(|(id, exists)| future::ready(exists.then_some(id)))
        .collect()
        .boxed()
        .await;

    debug!(count = all_conflicted.len(), "full conflicted set");
    trace!(set = ?all_conflicted, "full conflicted set");

    let control_events: Vec<_> = stream::iter(all_conflicted.iter())
        .map(|id| is_power_event_id(id, &event_fetch).map(move |is| (id, is)))
        .buffer_unordered(parallel_fetches)
        .filter_map(|(id, is)| future::ready(is.then_some(id.clone())))
        .collect()
        .boxed()
        .await;

    let sorted_control_levels = reverse_topological_power_sort(
        &room_version,
        control_events,
        &all_conflicted,
        &event_fetch,
        parallel_fetches,
    )
    .await?;

    debug!(count = sorted_control_levels.len(), "power events");
    trace!(list = ?sorted_control_levels, "sorted power events");

    // Version 2.1 starts the checks from an empty state map; the unconflicted
    // state still wins at the end.
    let initial_state = if room_version.state_res_v2_1 {
        StateMap::new()
    } else {
        clean.clone()
    };

    let resolved_control = iterative_auth_check(
        &room_version,
        sorted_control_levels.iter(),
        initial_state,
        &event_fetch,
        parallel_fetches,
    )
    .await?;

    debug!(count = resolved_control.len(), "resolved power events");
    trace!(map = ?resolved_control, "resolved power events");

    let deduped_power_ev = sorted_control_levels.into_iter().collect::<HashSet<_>>();

    let events_to_resolve = all_conflicted
        .iter()
        .filter(|&id| !deduped_power_ev.contains(id.borrow()))
        .cloned()
        .collect::<Vec<_>>();

    debug!(count = events_to_resolve.len(), "events left to resolve");
    trace!(list = ?events_to_resolve, "events left to resolve");

    let power_event = resolved_control.get(&(StateEventType::RoomPowerLevels, StateKey::new()));

    debug!(event_id = ?power_event, "power event");

    let sorted_left_events = mainline_sort(
        &events_to_resolve,
        power_event.cloned(),
        &event_fetch,
        parallel_fetches,
    )
    .await?;

    trace!(list = ?sorted_left_events, "events left, sorted");

    let mut resolved_state = iterative_auth_check(
        &room_version,
        sorted_left_events.iter(),
        resolved_control,
        &event_fetch,
        parallel_fetches,
    )
    .await?;

    resolved_state.extend(clean);

    debug!("state resolution finished");

    Ok(resolved_state)
}

/// The events on any `auth_events` path between two events of the conflicted
/// state set, endpoints included: the ancestors of conflicted events that are
/// themselves conflicted or reach a conflicted event.
async fn conflicted_state_subgraph<E, Fetch, FetchFut>(
    conflicted: &HashSet<E::Id>,
    event_fetch: &Fetch,
) -> HashSet<E::Id>
where
    Fetch: Fn(E::Id) -> FetchFut + Sync,
    FetchFut: Future<Output = Option<E>> + Send,
    E: Event + Send + Sync,
    E::Id: Borrow<EventId> + Send + Sync,
{
    let mut reaches: HashMap<E::Id, bool> = HashMap::new();
    let mut auth_of: HashMap<E::Id, Vec<E::Id>> = HashMap::new();
    let mut stack: Vec<(E::Id, bool)> = conflicted.iter().map(|id| (id.clone(), false)).collect();

    // Iterative post-order walk: an event's verdict needs its auth events'.
    while let Some((id, expanded)) = stack.pop() {
        if expanded {
            let key: &EventId = id.borrow();
            let reaches_conflicted = conflicted.contains(key)
                || auth_of.get(key).into_iter().flatten().any(|auth| {
                    let auth: &EventId = auth.borrow();
                    reaches.get(auth).copied().unwrap_or(false)
                });

            reaches.insert(id, reaches_conflicted);
            continue;
        }

        if auth_of.contains_key::<EventId>(id.borrow()) {
            continue;
        }

        let auth_events: Vec<E::Id> = event_fetch(id.clone())
            .await
            .map(|event| event.auth_events().cloned().collect())
            .unwrap_or_default();

        stack.push((id.clone(), true));
        stack.extend(
            auth_events
                .iter()
                .filter(|auth| !auth_of.contains_key::<EventId>((*auth).borrow()))
                .map(|auth| (auth.clone(), false)),
        );

        auth_of.insert(id, auth_events);
    }

    reaches
        .into_iter()
        .filter_map(|(id, reaches_conflicted)| reaches_conflicted.then_some(id))
        .collect()
}

fn separate<'a, Id>(
    state_sets_iter: impl Iterator<Item = &'a StateMap<Id>>,
) -> (StateMap<Id>, StateMap<Vec<Id>>)
where
    Id: Clone + Eq + Hash + 'a,
{
    let mut state_set_count: usize = 0;
    let mut occurrences = HashMap::<_, HashMap<_, _>>::new();

    let state_sets_iter =
        state_sets_iter.inspect(|_| state_set_count = state_set_count.saturating_add(1));
    for (k, v) in state_sets_iter.flatten() {
        occurrences
            .entry(k)
            .or_default()
            .entry(v)
            .and_modify(|x: &mut usize| *x = x.saturating_add(1))
            .or_insert(1);
    }

    let mut unconflicted_state = StateMap::new();
    let mut conflicted_state = StateMap::new();

    for (k, v) in occurrences {
        for (id, occurrence_count) in v {
            if occurrence_count == state_set_count {
                unconflicted_state.insert((k.0.clone(), k.1.clone()), id.clone());
            } else {
                conflicted_state
                    .entry((k.0.clone(), k.1.clone()))
                    .and_modify(|x: &mut Vec<_>| x.push(id.clone()))
                    .or_insert_with(|| vec![id.clone()]);
            }
        }
    }

    (unconflicted_state, conflicted_state)
}

#[allow(clippy::arithmetic_side_effects)]
fn get_auth_chain_diff<Id, Hasher>(
    auth_chain_sets: &[HashSet<Id, Hasher>],
) -> impl Iterator<Item = Id> + Send + use<Id, Hasher>
where
    Id: Clone + Eq + Hash + Send,
    Hasher: BuildHasher + Send + Sync,
{
    let num_sets = auth_chain_sets.len();
    let mut id_counts: HashMap<Id, usize> = HashMap::new();
    for id in auth_chain_sets.iter().flatten() {
        *id_counts.entry(id.clone()).or_default() += 1;
    }

    id_counts
        .into_iter()
        .filter_map(move |(id, count)| (count < num_sets).then_some(id))
}

/// The ID of the create event a room-version-12 room's ID is derived from:
/// the room ID's opaque part under a `$` sigil. `None` for a room ID that
/// carries a server name, which every older room version uses.
#[must_use]
pub fn create_event_id_of(room_id: &RoomId) -> Option<OwnedEventId> {
    if room_id.server_name().is_some() {
        return None;
    }

    let opaque = room_id.as_str().strip_prefix('!')?;
    EventId::parse(format!("${opaque}")).ok()
}

/// Whether `create_event` is the create event `room_id` was derived from.
fn room_id_derives_from(room_id: &RoomId, create_event: &impl Event) -> bool {
    create_event_id_of(room_id)
        .is_some_and(|id| id.as_str() == create_event.event_id().borrow().as_str())
}

fn is_type_and_key(ev: impl Event, ev_type: &TimelineEventType, state_key: &str) -> bool {
    ev.event_type() == ev_type && ev.state_key() == Some(state_key)
}

fn is_power_event(event: impl Event) -> bool {
    match event.event_type() {
        TimelineEventType::RoomPowerLevels
        | TimelineEventType::RoomJoinRules
        | TimelineEventType::RoomCreate => event.state_key() == Some(""),
        TimelineEventType::RoomMember => {
            if let Ok(content) = from_json_str::<RoomMemberEventContent>(event.content().get())
                && [MembershipState::Leave, MembershipState::Ban].contains(&content.membership)
            {
                return Some(event.sender().as_str()) != event.state_key();
            }

            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
