//! Benchmarks for state resolution.
//!
//! `#[bench]` is still nightly-only, so the whole module sits behind the
//! `phantom_bench` cfg and is not built by the pinned stable toolchain. Run it
//! with a nightly compiler:
//!
//! ```sh
//! RUSTFLAGS='--cfg phantom_bench' cargo +nightly bench -p phantom-core
//! ```
//!
//! The fixtures come from [`super::test_utils`] rather than being restated
//! here, so the benches and the tests exercise the same graphs.

extern crate test;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::{future, future::ready};
use maplit::{hashmap, hashset};
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId, RoomVersionId, events::TimelineEventType, int, uint,
};
use serde_json::{json, value::to_raw_value as to_raw_json_value};

use super::{
    StateMap, lexicographical_topological_sort, resolve,
    test_utils::{
        INITIAL_EVENTS, PduEvent, TestStore, alice, bob, ella, event_id, member_content_ban,
        member_content_join, room_id, to_pdu_event,
    },
};
use crate::matrix::{Event, EventTypeExt};

#[bench]
fn lexico_topo_sort(c: &mut test::Bencher) {
    let graph = hashmap! {
        event_id("l") => hashset![event_id("o")],
        event_id("m") => hashset![event_id("n"), event_id("o")],
        event_id("n") => hashset![event_id("o")],
        event_id("o") => hashset![], // "o" has zero outgoing edges but 4 incoming edges
        event_id("p") => hashset![event_id("o")],
    };

    c.iter(|| {
        let _ = lexicographical_topological_sort(&graph, &|_| {
            future::ok((int!(0), MilliSecondsSinceUnixEpoch(uint!(0))))
        });
    });
}

#[bench]
fn resolution_shallow_auth_chain(c: &mut test::Bencher) {
    let parallel_fetches = 32;
    let mut store = TestStore(hashmap! {});

    // build up the DAG
    let (state_at_bob, state_at_charlie, _) = store.set_up();

    c.iter(|| async {
        let ev_map = store.0.clone();
        let state_sets = [&state_at_bob, &state_at_charlie];
        let fetch = |id: OwnedEventId| ready(ev_map.get(&id).map(Arc::clone));
        let exists = |id: OwnedEventId| ready(ev_map.contains_key(&id));
        let auth_chain_sets: Vec<HashSet<_>> = state_sets
            .iter()
            .map(|map| {
                store
                    .auth_event_ids(room_id(), map.values().cloned().collect())
                    .unwrap()
            })
            .collect();

        match resolve(
            &RoomVersionId::V6,
            state_sets.into_iter(),
            &auth_chain_sets,
            &fetch,
            &exists,
            parallel_fetches,
        )
        .await
        {
            Ok(state) => state,
            Err(e) => panic!("{e}"),
        }
    });
}

#[bench]
fn resolve_deeper_event_set(c: &mut test::Bencher) {
    let parallel_fetches = 32;
    let mut inner = INITIAL_EVENTS();
    let ban = BAN_STATE_SET();

    inner.extend(ban);
    let store = TestStore(inner.clone());

    let state_set_a = state_map(&inner, &["CREATE", "IJR", "IMA", "IMB", "IMC", "MB", "PA"]);
    let state_set_b = state_map(&inner, &["CREATE", "IJR", "IMA", "IMB", "IMC", "IME", "PA"]);

    c.iter(|| async {
        let state_sets = [&state_set_a, &state_set_b];
        let auth_chain_sets: Vec<HashSet<_>> = state_sets
            .iter()
            .map(|map| {
                store
                    .auth_event_ids(room_id(), map.values().cloned().collect())
                    .unwrap()
            })
            .collect();

        let fetch = |id: OwnedEventId| ready(inner.get(&id).map(Arc::clone));
        let exists = |id: OwnedEventId| ready(inner.contains_key(&id));

        match resolve(
            &RoomVersionId::V6,
            state_sets.into_iter(),
            &auth_chain_sets,
            &fetch,
            &exists,
            parallel_fetches,
        )
        .await
        {
            Ok(state) => state,
            Err(_) => panic!("resolution failed during benchmarking"),
        }
    });
}

/// Collect the named events out of `events` into the state map shape `resolve`
/// takes.
fn state_map(
    events: &HashMap<OwnedEventId, Arc<PduEvent>>,
    ids: &[&str],
) -> StateMap<OwnedEventId> {
    ids.iter()
        .map(|id| {
            events
                .get(&event_id(id))
                .unwrap_or_else(|| panic!("{id} is not one of the fixture events"))
        })
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                ev.event_id().to_owned(),
            )
        })
        .collect()
}

/// A power level change plus a ban, layered on top of [`INITIAL_EVENTS`].
#[allow(non_snake_case)]
fn BAN_STATE_SET() -> HashMap<OwnedEventId, Arc<PduEvent>> {
    vec![
        to_pdu_event(
            "PA",
            alice(),
            TimelineEventType::RoomPowerLevels,
            Some(""),
            to_raw_json_value(&json!({ "users": { alice(): 100, bob(): 50 } })).unwrap(),
            &["CREATE", "IMA", "IPOWER"], // auth_events
            &["START"],                   // prev_events
        ),
        to_pdu_event(
            "PB",
            alice(),
            TimelineEventType::RoomPowerLevels,
            Some(""),
            to_raw_json_value(&json!({ "users": { alice(): 100, bob(): 50 } })).unwrap(),
            &["CREATE", "IMA", "IPOWER"],
            &["END"],
        ),
        to_pdu_event(
            "MB",
            alice(),
            TimelineEventType::RoomMember,
            Some(ella().as_str()),
            member_content_ban(),
            &["CREATE", "IMA", "PB"],
            &["PA"],
        ),
        to_pdu_event(
            "IME",
            ella(),
            TimelineEventType::RoomMember,
            Some(ella().as_str()),
            member_content_join(),
            &["CREATE", "IJR", "PA"],
            &["MB"],
        ),
    ]
    .into_iter()
    .map(|ev| (ev.event_id().to_owned(), ev))
    .collect()
}
