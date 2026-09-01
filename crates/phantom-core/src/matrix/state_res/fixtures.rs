use std::{
    borrow::Borrow,
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::SeqCst},
    },
};

use futures::future::ready;
use ruma::{
    EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, RoomId, RoomVersionId, ServerSignatures,
    UserId, event_id,
    events::{
        TimelineEventType,
        room::{
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            member::{MembershipState, RoomMemberEventContent},
        },
    },
    int, room_id, uint, user_id,
};
use serde_json::{
    json,
    value::{RawValue as RawJsonValue, to_raw_value as to_raw_json_value},
};

pub(crate) use self::event::{EventHash, PduEvent, RoomV3Pdu};
use super::auth_types_for_event;
use crate::{
    Result, info,
    matrix::{Event, EventTypeExt, StateMap},
};

static SERVER_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn do_check(
    events: &[Arc<PduEvent>],
    edges: Vec<Vec<OwnedEventId>>,
    expected_state_ids: Vec<OwnedEventId>,
) {
    let init_events = INITIAL_EVENTS();

    let mut store = TestStore(
        init_events
            .values()
            .chain(events)
            .map(|ev| (ev.event_id().to_owned(), ev.clone()))
            .collect(),
    );

    let mut graph = HashMap::new();
    let mut fake_event_map = HashMap::new();

    for ev in init_events.values().chain(events) {
        graph.insert(ev.event_id().to_owned(), HashSet::new());
        fake_event_map.insert(ev.event_id().to_owned(), ev.clone());
    }

    for pair in INITIAL_EDGES().windows(2) {
        if let [a, b] = &pair {
            graph
                .entry(a.to_owned())
                .or_insert_with(HashSet::new)
                .insert(b.clone());
        }
    }

    for edge_list in edges {
        for pair in edge_list.windows(2) {
            if let [a, b] = &pair {
                graph
                    .entry(a.to_owned())
                    .or_insert_with(HashSet::new)
                    .insert(b.clone());
            }
        }
    }

    let mut event_map: HashMap<OwnedEventId, Arc<PduEvent>> = HashMap::new();
    let mut state_at_event: HashMap<OwnedEventId, StateMap<OwnedEventId>> = HashMap::new();

    for node in super::lexicographical_topological_sort(&graph, &|_id| async {
        Ok((int!(0), MilliSecondsSinceUnixEpoch(uint!(0))))
    })
    .await
    .unwrap()
    {
        let fake_event = fake_event_map.get(&node).unwrap();
        let event_id = fake_event.event_id().to_owned();

        let prev_events = graph.get(&node).unwrap();

        let state_before: StateMap<OwnedEventId> = if prev_events.is_empty() {
            HashMap::new()
        } else if prev_events.len() == 1 {
            state_at_event
                .get(prev_events.iter().next().unwrap())
                .unwrap()
                .clone()
        } else {
            let state_sets = prev_events
                .iter()
                .filter_map(|k| state_at_event.get(k))
                .collect::<Vec<_>>();

            info!(
                "{:#?}",
                state_sets
                    .iter()
                    .map(|map| map
                        .iter()
                        .map(|((ty, key), id)| format!("(({ty}{key:?}), {id})"))
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );

            let auth_chain_sets: Vec<_> = state_sets
                .iter()
                .map(|map| {
                    store
                        .auth_event_ids(room_id(), map.values().cloned().collect())
                        .unwrap()
                })
                .collect();

            let event_map = &event_map;
            let fetch = |id: <PduEvent as Event>::Id| ready(event_map.get(&id).cloned());
            let exists = |id: <PduEvent as Event>::Id| ready(event_map.get(&id).is_some());
            let resolved = super::resolve(
                &RoomVersionId::V6,
                state_sets,
                &auth_chain_sets,
                &fetch,
                &exists,
                1,
            )
            .await;

            match resolved {
                Ok(state) => state,
                Err(e) => panic!("resolution for {node} failed: {e}"),
            }
        };

        let mut state_after = state_before.clone();

        let ty = fake_event.event_type();
        let key = fake_event.state_key().unwrap();
        state_after.insert(ty.with_state_key(key), event_id.to_owned());

        let auth_types = auth_types_for_event(
            fake_event.event_type(),
            fake_event.sender(),
            fake_event.state_key(),
            fake_event.content(),
        )
        .unwrap();

        let mut auth_events = vec![];
        for key in auth_types {
            if state_before.contains_key(&key) {
                auth_events.push(state_before[&key].clone());
            }
        }

        let e = fake_event;
        let ev_id = e.event_id();
        let event = to_pdu_event(
            e.event_id().as_str(),
            e.sender(),
            e.event_type().clone(),
            e.state_key(),
            e.content().to_owned(),
            &auth_events,
            &prev_events.iter().cloned().collect::<Vec<_>>(),
        );

        store.0.insert(ev_id.to_owned(), event.clone());

        state_at_event.insert(node, state_after);
        event_map.insert(event_id.to_owned(), Arc::clone(store.0.get(ev_id).unwrap()));
    }

    let mut expected_state = StateMap::new();
    for node in expected_state_ids {
        let ev = event_map.get(&node).unwrap_or_else(|| {
            panic!(
                "{node} not found in {:?}",
                event_map
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            )
        });

        let key = ev.event_type().with_state_key(ev.state_key().unwrap());

        expected_state.insert(key, node);
    }

    let start_state = state_at_event.get(event_id!("$START:foo")).unwrap();

    let end_state = state_at_event
        .get(event_id!("$END:foo"))
        .unwrap()
        .iter()
        .filter(|(k, v)| {
            expected_state.contains_key(k)
                || start_state.get(k) != Some(*v)
                    && **k != ("m.room.message".into(), "dummy".into())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<StateMap<OwnedEventId>>();

    assert_eq!(expected_state, end_state);
}

#[allow(clippy::exhaustive_structs)]
pub(crate) struct TestStore<E: Event>(pub(crate) HashMap<OwnedEventId, Arc<E>>);

impl<E: Event> TestStore<E> {
    pub(crate) fn get_event(&self, _: &RoomId, event_id: &EventId) -> Result<Arc<E>> {
        self.0
            .get(event_id)
            .cloned()
            .ok_or_else(|| super::Error::NotFound(format!("{event_id} not found")))
            .map_err(Into::into)
    }

    /// Returns a Vec of the related auth events to the given `event`.
    pub(crate) fn auth_event_ids(
        &self,
        room_id: &RoomId,
        event_ids: Vec<E::Id>,
    ) -> Result<HashSet<E::Id>> {
        let mut result = HashSet::new();
        let mut stack = event_ids;

        while let Some(ev_id) = stack.pop() {
            if result.contains(&ev_id) {
                continue;
            }

            result.insert(ev_id.clone());

            let event = self.get_event(room_id, ev_id.borrow())?;

            stack.extend(event.auth_events().map(ToOwned::to_owned));
        }

        Ok(result)
    }
}

#[allow(clippy::type_complexity)]
impl TestStore<PduEvent> {
    pub(crate) fn set_up(
        &mut self,
    ) -> (
        StateMap<OwnedEventId>,
        StateMap<OwnedEventId>,
        StateMap<OwnedEventId>,
    ) {
        let create_event = to_pdu_event::<&EventId>(
            "CREATE",
            alice(),
            TimelineEventType::RoomCreate,
            Some(""),
            to_raw_json_value(&json!({ "creator": alice() })).unwrap(),
            &[],
            &[],
        );
        let cre = create_event.event_id().to_owned();
        self.0.insert(cre.clone(), Arc::clone(&create_event));

        let alice_mem = to_pdu_event(
            "IMA",
            alice(),
            TimelineEventType::RoomMember,
            Some(alice().as_str()),
            member_content_join(),
            &[cre.clone()],
            &[cre.clone()],
        );
        self.0
            .insert(alice_mem.event_id().to_owned(), Arc::clone(&alice_mem));

        let join_rules = to_pdu_event(
            "IJR",
            alice(),
            TimelineEventType::RoomJoinRules,
            Some(""),
            to_raw_json_value(&RoomJoinRulesEventContent::new(JoinRule::Public)).unwrap(),
            &[cre.clone(), alice_mem.event_id().to_owned()],
            &[alice_mem.event_id().to_owned()],
        );
        self.0
            .insert(join_rules.event_id().to_owned(), join_rules.clone());

        let bob_mem = to_pdu_event(
            "IMB",
            bob(),
            TimelineEventType::RoomMember,
            Some(bob().as_str()),
            member_content_join(),
            &[cre.clone(), join_rules.event_id().to_owned()],
            &[join_rules.event_id().to_owned()],
        );
        self.0
            .insert(bob_mem.event_id().to_owned(), bob_mem.clone());

        let charlie_mem = to_pdu_event(
            "IMC",
            charlie(),
            TimelineEventType::RoomMember,
            Some(charlie().as_str()),
            member_content_join(),
            &[cre, join_rules.event_id().to_owned()],
            &[join_rules.event_id().to_owned()],
        );
        self.0
            .insert(charlie_mem.event_id().to_owned(), charlie_mem.clone());

        let state_at_bob = [&create_event, &alice_mem, &join_rules, &bob_mem]
            .iter()
            .map(|e| {
                (
                    e.event_type().with_state_key(e.state_key().unwrap()),
                    e.event_id().to_owned(),
                )
            })
            .collect::<StateMap<_>>();

        let state_at_charlie = [&create_event, &alice_mem, &join_rules, &charlie_mem]
            .iter()
            .map(|e| {
                (
                    e.event_type().with_state_key(e.state_key().unwrap()),
                    e.event_id().to_owned(),
                )
            })
            .collect::<StateMap<_>>();

        let expected = [
            &create_event,
            &alice_mem,
            &join_rules,
            &bob_mem,
            &charlie_mem,
        ]
        .iter()
        .map(|e| {
            (
                e.event_type().with_state_key(e.state_key().unwrap()),
                e.event_id().to_owned(),
            )
        })
        .collect::<StateMap<_>>();

        (state_at_bob, state_at_charlie, expected)
    }
}

pub(crate) fn event_id(id: &str) -> OwnedEventId {
    if id.contains('$') {
        return id.try_into().unwrap();
    }

    format!("${id}:foo").try_into().unwrap()
}

pub(crate) fn alice() -> &'static UserId {
    user_id!("@alice:foo")
}

pub(crate) fn bob() -> &'static UserId {
    user_id!("@bob:foo")
}

pub(crate) fn charlie() -> &'static UserId {
    user_id!("@charlie:foo")
}

pub(crate) fn ella() -> &'static UserId {
    user_id!("@ella:foo")
}

pub(crate) fn zara() -> &'static UserId {
    user_id!("@zara:foo")
}

pub(crate) fn room_id() -> &'static RoomId {
    room_id!("!test:foo")
}

pub(crate) fn member_content_ban() -> Box<RawJsonValue> {
    to_raw_json_value(&RoomMemberEventContent::new(MembershipState::Ban)).unwrap()
}

pub(crate) fn member_content_join() -> Box<RawJsonValue> {
    to_raw_json_value(&RoomMemberEventContent::new(MembershipState::Join)).unwrap()
}

pub(crate) fn to_init_pdu_event(
    id: &str,
    sender: &UserId,
    ev_type: TimelineEventType,
    state_key: Option<&str>,
    content: Box<RawJsonValue>,
) -> Arc<PduEvent> {
    let ts = SERVER_TIMESTAMP.fetch_add(1, SeqCst);
    let id = if id.contains('$') {
        id.to_owned()
    } else {
        format!("${id}:foo")
    };

    let state_key = state_key.map(ToOwned::to_owned);
    Arc::new(PduEvent {
        event_id: id.try_into().unwrap(),
        rest: RoomV3Pdu {
            room_id: room_id().to_owned(),
            sender: sender.to_owned(),
            origin_server_ts: MilliSecondsSinceUnixEpoch(ts.try_into().unwrap()),
            state_key,
            kind: ev_type,
            content,
            redacts: None,
            unsigned: BTreeMap::new(),
            auth_events: vec![],
            prev_events: vec![],
            depth: uint!(0),
            hashes: EventHash::new("".to_owned()),
            signatures: ServerSignatures::default(),
        },
    })
}

pub(crate) fn to_pdu_event<S>(
    id: &str,
    sender: &UserId,
    ev_type: TimelineEventType,
    state_key: Option<&str>,
    content: Box<RawJsonValue>,
    auth_events: &[S],
    prev_events: &[S],
) -> Arc<PduEvent>
where
    S: AsRef<str>,
{
    let ts = SERVER_TIMESTAMP.fetch_add(1, SeqCst);
    let id = if id.contains('$') {
        id.to_owned()
    } else {
        format!("${id}:foo")
    };
    let auth_events = auth_events
        .iter()
        .map(AsRef::as_ref)
        .map(event_id)
        .collect::<Vec<_>>();
    let prev_events = prev_events
        .iter()
        .map(AsRef::as_ref)
        .map(event_id)
        .collect::<Vec<_>>();

    let state_key = state_key.map(ToOwned::to_owned);
    Arc::new(PduEvent {
        event_id: id.try_into().unwrap(),
        rest: RoomV3Pdu {
            room_id: room_id().to_owned(),
            sender: sender.to_owned(),
            origin_server_ts: MilliSecondsSinceUnixEpoch(ts.try_into().unwrap()),
            state_key,
            kind: ev_type,
            content,
            redacts: None,
            unsigned: BTreeMap::new(),
            auth_events,
            prev_events,
            depth: uint!(0),
            hashes: EventHash::new("".to_owned()),
            signatures: ServerSignatures::default(),
        },
    })
}

#[allow(non_snake_case)]
pub(crate) fn INITIAL_EVENTS() -> HashMap<OwnedEventId, Arc<PduEvent>> {
    vec![
        to_pdu_event::<&EventId>(
            "CREATE",
            alice(),
            TimelineEventType::RoomCreate,
            Some(""),
            to_raw_json_value(&json!({ "creator": alice() })).unwrap(),
            &[],
            &[],
        ),
        to_pdu_event(
            "IMA",
            alice(),
            TimelineEventType::RoomMember,
            Some(alice().as_str()),
            member_content_join(),
            &["CREATE"],
            &["CREATE"],
        ),
        to_pdu_event(
            "IPOWER",
            alice(),
            TimelineEventType::RoomPowerLevels,
            Some(""),
            to_raw_json_value(&json!({ "users": { alice(): 100 } })).unwrap(),
            &["CREATE", "IMA"],
            &["IMA"],
        ),
        to_pdu_event(
            "IJR",
            alice(),
            TimelineEventType::RoomJoinRules,
            Some(""),
            to_raw_json_value(&RoomJoinRulesEventContent::new(JoinRule::Public)).unwrap(),
            &["CREATE", "IMA", "IPOWER"],
            &["IPOWER"],
        ),
        to_pdu_event(
            "IMB",
            bob(),
            TimelineEventType::RoomMember,
            Some(bob().as_str()),
            member_content_join(),
            &["CREATE", "IJR", "IPOWER"],
            &["IJR"],
        ),
        to_pdu_event(
            "IMC",
            charlie(),
            TimelineEventType::RoomMember,
            Some(charlie().as_str()),
            member_content_join(),
            &["CREATE", "IJR", "IPOWER"],
            &["IMB"],
        ),
        to_pdu_event::<&EventId>(
            "START",
            charlie(),
            TimelineEventType::RoomMessage,
            Some("dummy"),
            to_raw_json_value(&json!({})).unwrap(),
            &[],
            &[],
        ),
        to_pdu_event::<&EventId>(
            "END",
            charlie(),
            TimelineEventType::RoomMessage,
            Some("dummy"),
            to_raw_json_value(&json!({})).unwrap(),
            &[],
            &[],
        ),
    ]
    .into_iter()
    .map(|ev| (ev.event_id().to_owned(), ev))
    .collect()
}

#[allow(non_snake_case)]
pub(crate) fn INITIAL_EVENTS_CREATE_ROOM() -> HashMap<OwnedEventId, Arc<PduEvent>> {
    vec![to_pdu_event::<&EventId>(
        "CREATE",
        alice(),
        TimelineEventType::RoomCreate,
        Some(""),
        to_raw_json_value(&json!({ "creator": alice() })).unwrap(),
        &[],
        &[],
    )]
    .into_iter()
    .map(|ev| (ev.event_id().to_owned(), ev))
    .collect()
}

#[allow(non_snake_case)]
pub(crate) fn INITIAL_EDGES() -> Vec<OwnedEventId> {
    vec!["START", "IMC", "IMB", "IJR", "IPOWER", "IMA", "CREATE"]
        .into_iter()
        .map(event_id)
        .collect::<Vec<_>>()
}

pub(crate) mod event {
    use std::collections::BTreeMap;

    use ruma::{
        MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId,
        ServerSignatures, UInt, UserId, events::TimelineEventType,
    };
    use serde::{Deserialize, Serialize};
    use serde_json::value::{RawValue as RawJsonValue, Value as JsonValue};

    use crate::matrix::Event;

    /// ruma no longer ships the `events::pdu` types these tests were built on.
    ///
    /// Only the room v3 shape was ever constructed here, so this is that one
    /// struct rather than the enum ruma used to expose.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub(crate) struct RoomV3Pdu {
        pub(crate) room_id: OwnedRoomId,
        pub(crate) sender: OwnedUserId,
        pub(crate) origin_server_ts: MilliSecondsSinceUnixEpoch,
        #[serde(rename = "type")]
        pub(crate) kind: TimelineEventType,
        pub(crate) content: Box<RawJsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub(crate) state_key: Option<String>,
        pub(crate) prev_events: Vec<OwnedEventId>,
        pub(crate) depth: UInt,
        pub(crate) auth_events: Vec<OwnedEventId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub(crate) redacts: Option<OwnedEventId>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub(crate) unsigned: BTreeMap<String, JsonValue>,
        pub(crate) hashes: EventHash,
        pub(crate) signatures: ServerSignatures,
    }

    /// Content hashes of a PDU.
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub(crate) struct EventHash {
        pub(crate) sha256: String,
    }

    impl EventHash {
        pub(crate) fn new(sha256: String) -> Self {
            Self { sha256 }
        }
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub(crate) struct PduEvent {
        pub(crate) event_id: OwnedEventId,
        #[serde(flatten)]
        pub(crate) rest: RoomV3Pdu,
    }

    impl Event for PduEvent {
        type Id = OwnedEventId;

        fn event_id(&self) -> &Self::Id {
            &self.event_id
        }

        fn room_id(&self) -> &RoomId {
            &self.rest.room_id
        }

        fn sender(&self) -> &UserId {
            &self.rest.sender
        }

        fn event_type(&self) -> &TimelineEventType {
            &self.rest.kind
        }

        fn content(&self) -> &RawJsonValue {
            &self.rest.content
        }

        fn origin_server_ts(&self) -> MilliSecondsSinceUnixEpoch {
            self.rest.origin_server_ts
        }

        fn state_key(&self) -> Option<&str> {
            self.rest.state_key.as_deref()
        }

        fn prev_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
            self.rest.prev_events.iter()
        }

        fn auth_events(&self) -> impl DoubleEndedIterator<Item = &Self::Id> + Send + '_ {
            self.rest.auth_events.iter()
        }

        fn redacts(&self) -> Option<&Self::Id> {
            self.rest.redacts.as_ref()
        }
    }
}
