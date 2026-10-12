//! Room version 12: creators outrank every power level, and the room ID names
//! the create event instead of the auth events listing it.

use std::sync::Arc;

use futures::future::ready;
use ruma::{
    OwnedRoomId, RoomId,
    events::{
        StateEventType, TimelineEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};
use serde_json::{json, value::to_raw_value as to_raw_json_value};

use crate::matrix::state_res::{
    Event, EventTypeExt, RoomVersion, StateMap, auth_check, create_event_id_of,
    fixtures::{PduEvent, alice, bob, charlie, to_pdu_event},
};

const CREATE: &str = "$v12createevent";

fn v12_room() -> OwnedRoomId {
    RoomId::parse("!v12createevent").unwrap()
}

/// An event in the v12 room; `to_pdu_event` puts it in the fixtures' room.
fn event(
    id: &str,
    sender: &ruma::UserId,
    kind: TimelineEventType,
    state_key: Option<&str>,
    content: serde_json::Value,
    auth_events: &[&str],
) -> Arc<PduEvent> {
    let mut pdu = (*to_pdu_event(
        id,
        sender,
        kind,
        state_key,
        to_raw_json_value(&content).unwrap(),
        auth_events,
        &["$prev"],
    ))
    .clone();
    pdu.rest.room_id = v12_room();
    Arc::new(pdu)
}

fn member(user: &ruma::UserId) -> Arc<PduEvent> {
    event(
        &format!("$member_{}", user.localpart()),
        user,
        TimelineEventType::RoomMember,
        Some(user.as_str()),
        json!({ "membership": "join" }),
        &[],
    )
}

/// The room's state: alice created it naming bob as an additional creator,
/// charlie holds 100 through power levels, and all three are joined.
fn state() -> StateMap<Arc<PduEvent>> {
    let mut create = (*event(
        CREATE,
        alice(),
        TimelineEventType::RoomCreate,
        Some(""),
        json!({ "room_version": "12", "additional_creators": [bob()] }),
        &[],
    ))
    .clone();
    create.rest.prev_events.clear();

    let power_levels = event(
        "$power_levels",
        alice(),
        TimelineEventType::RoomPowerLevels,
        Some(""),
        json!({ "users": { charlie(): 100 } }),
        &[],
    );

    [
        Arc::new(create),
        power_levels,
        member(alice()),
        member(bob()),
        member(charlie()),
    ]
    .into_iter()
    .map(|ev| (ev.event_type().with_state_key(ev.state_key().unwrap()), ev))
    .collect()
}

async fn allowed(incoming: &Arc<PduEvent>, state: &StateMap<Arc<PduEvent>>) -> bool {
    auth_check(
        &RoomVersion::V12,
        incoming,
        None,
        |ty: &StateEventType, key: &str| ready(state.get(&ty.with_state_key(key)).cloned()),
    )
    .await
    .unwrap()
}

fn kick(sender: &ruma::UserId, target: &ruma::UserId) -> Arc<PduEvent> {
    event(
        "$kick",
        sender,
        TimelineEventType::RoomMember,
        Some(target.as_str()),
        serde_json::to_value(RoomMemberEventContent::new(MembershipState::Leave)).unwrap(),
        &["$power_levels", &format!("$member_{}", sender.localpart())],
    )
}

#[test]
fn the_create_event_id_is_derived_from_the_room_id() {
    assert_eq!(create_event_id_of(&v12_room()).unwrap().as_str(), CREATE);
    assert!(create_event_id_of(ruma::room_id!("!legacy:example.org")).is_none());
}

#[tokio::test]
async fn an_additional_creator_outranks_a_power_level_of_100() {
    let state = state();

    assert!(allowed(&kick(bob(), charlie()), &state).await);
    assert!(!allowed(&kick(charlie(), bob()), &state).await);
}

#[tokio::test]
async fn creators_cannot_kick_each_other() {
    assert!(!allowed(&kick(alice(), bob()), &state()).await);
}

#[tokio::test]
async fn power_levels_may_not_list_a_creator() {
    let state = state();
    let listing = |users: serde_json::Value| {
        event(
            "$new_power_levels",
            alice(),
            TimelineEventType::RoomPowerLevels,
            Some(""),
            json!({ "users": users }),
            &["$power_levels", "$member_alice"],
        )
    };

    assert!(allowed(&listing(json!({ charlie(): 50 })), &state).await);
    assert!(!allowed(&listing(json!({ bob(): 100 })), &state).await);
}

#[tokio::test]
async fn the_create_event_must_not_be_an_auth_event() {
    let state = state();
    let message = |auth_events: &[&str]| {
        event(
            "$message",
            charlie(),
            TimelineEventType::RoomMessage,
            None,
            json!({ "body": "hi", "msgtype": "m.text" }),
            auth_events,
        )
    };

    assert!(allowed(&message(&["$power_levels", "$member_charlie"]), &state).await);
    assert!(
        !allowed(
            &message(&[CREATE, "$power_levels", "$member_charlie"]),
            &state
        )
        .await
    );
}

#[tokio::test]
async fn an_event_must_be_in_the_room_its_create_event_names() {
    let state = state();
    let mut stray = (*kick(bob(), charlie())).clone();
    stray.rest.room_id = RoomId::parse("!someotherroom").unwrap();

    assert!(!allowed(&Arc::new(stray), &state).await);
}

#[tokio::test]
async fn additional_creators_must_be_user_ids() {
    let create = |additional: serde_json::Value| {
        let mut create = (*event(
            CREATE,
            alice(),
            TimelineEventType::RoomCreate,
            Some(""),
            json!({ "room_version": "12", "additional_creators": additional }),
            &[],
        ))
        .clone();
        create.rest.prev_events.clear();
        Arc::new(create)
    };
    let empty = StateMap::new();

    assert!(allowed(&create(json!([bob()])), &empty).await);
    assert!(!allowed(&create(json!(["not a user"])), &empty).await);
}
