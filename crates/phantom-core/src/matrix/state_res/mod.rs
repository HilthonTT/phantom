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
    EventId, Int, MilliSecondsSinceUnixEpoch, RoomVersionId,
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

/// A mapping of event type and state_key to some value `T`, usually an
/// `EventId`.
pub type StateMap<T> = HashMap<TypeStateKey, T>;
pub type StateMapItem<T> = (TypeStateKey, T);
pub type TypeStateKey = (StateEventType, StateKey);

type Result<T, E = Error> = crate::Result<T, E>;

/// Resolve sets of state events as they come in.
///
/// Internally `StateResolution` builds a graph and an auth chain to allow for
/// state conflict resolution.
///
/// ## Arguments
///
/// * `state_sets` - The incoming state to resolve. Each `StateMap` represents a
///   possible fork in the state of a room.
///
/// * `auth_chain_sets` - The full recursive set of `auth_events` for each event
///   in the `state_sets`.
///
/// * `event_fetch` - Any event not found in the `event_map` will defer to this
///   closure to find the event.
///
/// * `parallel_fetches` - The number of asynchronous fetch requests in-flight
///   for any given operation.
///
/// ## Invariants
///
/// The caller of `resolve` must ensure that all the events are from the same
/// room. Although this function takes a `RoomId` it does not check that each
/// event is part of the same room.
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
    E::Id: Borrow<EventId> + Send + Sync,
    for<'b> &'b E: Send,
{
    debug!("State resolution starting");

    let (clean, conflicting) = separate(state_sets.into_iter());

    debug!(count = clean.len(), "non-conflicting events");
    trace!(map = ?clean, "non-conflicting events");

    if conflicting.is_empty() {
        debug!("no conflicting state found");
        return Ok(clean);
    }

    debug!(count = conflicting.len(), "conflicting events");
    trace!(map = ?conflicting, "conflicting events");

    let auth_chain_diff =
        get_auth_chain_diff(auth_chain_sets).chain(conflicting.into_values().flatten());

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
        control_events,
        &all_conflicted,
        &event_fetch,
        parallel_fetches,
    )
    .await?;

    debug!(count = sorted_control_levels.len(), "power events");
    trace!(list = ?sorted_control_levels, "sorted power events");

    let room_version = RoomVersion::new(room_version)?;
    let resolved_control = iterative_auth_check(
        &room_version,
        sorted_control_levels.iter(),
        clean.clone(),
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

/// Split the events that have no conflicts from those that are conflicting.
///
/// The return tuple looks like `(unconflicted, conflicted)`.
///
/// State is determined to be conflicting if for the given key (StateEventType,
/// StateKey) there is not exactly one event ID. This includes missing events,
/// if one state_set includes an event that none of the other have this is a
/// conflicting event.
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

/// Returns a Vec of deduped EventIds that appear in some chains but not others.
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
