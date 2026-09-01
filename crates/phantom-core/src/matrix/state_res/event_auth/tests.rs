use std::sync::Arc;

use ruma::events::{
    StateEventType, TimelineEventType,
    room::{
        join_rules::{AllowRule, JoinRule, Restricted, RoomJoinRulesEventContent},
        member::{MembershipState, RoomMemberEventContent},
    },
};
use serde_json::value::to_raw_value as to_raw_json_value;

use crate::matrix::state_res::{
    Event, EventTypeExt, RoomVersion, StateMap,
    event_auth::valid_membership_change,
    fixtures::{
        INITIAL_EVENTS, INITIAL_EVENTS_CREATE_ROOM, PduEvent, alice, charlie, ella, event_id,
        member_content_ban, member_content_join, room_id, to_pdu_event,
    },
};

#[test]
fn test_ban_pass() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let events = INITIAL_EVENTS();

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        alice(),
        TimelineEventType::RoomMember,
        Some(charlie().as_str()),
        member_content_ban(),
        &[],
        &["IMC"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = charlie();
    let sender = alice();

    assert!(
        valid_membership_change(
            &RoomVersion::V6,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            None,
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}

#[test]
fn test_join_non_creator() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let events = INITIAL_EVENTS_CREATE_ROOM();

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        charlie(),
        TimelineEventType::RoomMember,
        Some(charlie().as_str()),
        member_content_join(),
        &["CREATE"],
        &["CREATE"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = charlie();
    let sender = charlie();

    assert!(
        !valid_membership_change(
            &RoomVersion::V6,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            None,
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}

#[test]
fn test_join_creator() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let events = INITIAL_EVENTS_CREATE_ROOM();

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        alice(),
        TimelineEventType::RoomMember,
        Some(alice().as_str()),
        member_content_join(),
        &["CREATE"],
        &["CREATE"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = alice();
    let sender = alice();

    assert!(
        valid_membership_change(
            &RoomVersion::V6,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            None,
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}

#[test]
fn test_ban_fail() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let events = INITIAL_EVENTS();

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        charlie(),
        TimelineEventType::RoomMember,
        Some(alice().as_str()),
        member_content_ban(),
        &[],
        &["IMC"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = alice();
    let sender = charlie();

    assert!(
        !valid_membership_change(
            &RoomVersion::V6,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            None,
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}

#[test]
fn test_restricted_join_rule() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let mut events = INITIAL_EVENTS();
    *events.get_mut(&event_id("IJR")).unwrap() = to_pdu_event(
        "IJR",
        alice(),
        TimelineEventType::RoomJoinRules,
        Some(""),
        to_raw_json_value(&RoomJoinRulesEventContent::new(JoinRule::Restricted(
            Restricted::new(vec![AllowRule::room_membership(room_id().to_owned())]),
        )))
        .unwrap(),
        &["CREATE", "IMA", "IPOWER"],
        &["IPOWER"],
    );

    let mut member = RoomMemberEventContent::new(MembershipState::Join);
    member.join_authorized_via_users_server = Some(alice().to_owned());

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        ella(),
        TimelineEventType::RoomMember,
        Some(ella().as_str()),
        to_raw_json_value(&RoomMemberEventContent::new(MembershipState::Join)).unwrap(),
        &["CREATE", "IJR", "IPOWER", "new"],
        &["new"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = ella();
    let sender = ella();

    assert!(
        valid_membership_change(
            &RoomVersion::V9,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            Some(alice()),
            &MembershipState::Join,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );

    assert!(
        !valid_membership_change(
            &RoomVersion::V9,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            Some(ella()),
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}

#[test]
fn test_knock() {
    let _ = tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish());
    let mut events = INITIAL_EVENTS();
    *events.get_mut(&event_id("IJR")).unwrap() = to_pdu_event(
        "IJR",
        alice(),
        TimelineEventType::RoomJoinRules,
        Some(""),
        to_raw_json_value(&RoomJoinRulesEventContent::new(JoinRule::Knock)).unwrap(),
        &["CREATE", "IMA", "IPOWER"],
        &["IPOWER"],
    );

    let auth_events = events
        .values()
        .map(|ev| {
            (
                ev.event_type().with_state_key(ev.state_key().unwrap()),
                Arc::clone(ev),
            )
        })
        .collect::<StateMap<_>>();

    let requester = to_pdu_event(
        "HELLO",
        ella(),
        TimelineEventType::RoomMember,
        Some(ella().as_str()),
        to_raw_json_value(&RoomMemberEventContent::new(MembershipState::Knock)).unwrap(),
        &[],
        &["IMC"],
    );

    let fetch_state = |ty, key| auth_events.get(&(ty, key)).cloned();
    let target_user = ella();
    let sender = ella();

    assert!(
        valid_membership_change(
            &RoomVersion::V7,
            target_user,
            fetch_state(StateEventType::RoomMember, target_user.as_str().into()).as_ref(),
            sender,
            fetch_state(StateEventType::RoomMember, sender.as_str().into()).as_ref(),
            &requester,
            None::<&PduEvent>,
            fetch_state(StateEventType::RoomPowerLevels, "".into()).as_ref(),
            fetch_state(StateEventType::RoomJoinRules, "".into()).as_ref(),
            None,
            &MembershipState::Leave,
            &fetch_state(StateEventType::RoomCreate, "".into()).unwrap(),
        )
        .unwrap()
    );
}
