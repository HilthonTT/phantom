use ruma::{
    OwnedServerName, ServerName, UserId,
    api::appservice::{Namespace, Namespaces, Registration, RegistrationInit},
};

use super::{RegistrationInfo, Registrations, check_collisions, exclusive_overlap};

fn server_name() -> OwnedServerName {
    ServerName::parse("phantom.test").expect("valid server name")
}

/// A registration with one exclusive user namespace, which is the shape
/// almost every appservice has.
fn registration(id: &str, sender: &str, token: &str, users: Vec<Namespace>) -> RegistrationInfo {
    let mut namespaces = Namespaces::new();
    namespaces.users = users;

    let registration: Registration = RegistrationInit {
        id: id.to_owned(),
        url: Some("http://localhost:1234".to_owned()),
        as_token: format!("as_{token}"),
        hs_token: format!("hs_{token}"),
        sender_localpart: sender.to_owned(),
        namespaces,
        rate_limited: None,
        protocols: None,
    }
    .into();

    registration.try_into().expect("namespaces compile")
}

fn exclusive_users(regex: &str) -> Vec<Namespace> {
    vec![Namespace::new(true, regex.to_owned())]
}

fn registered(infos: impl IntoIterator<Item = RegistrationInfo>) -> Registrations {
    infos
        .into_iter()
        .map(|info| (info.registration.id.clone(), info))
        .collect()
}

#[test]
fn namespaces_match_only_what_they_cover() {
    let info = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));

    let claimed = UserId::parse("@irc_alice:phantom.test").expect("valid user id");
    let other = UserId::parse("@alice:phantom.test").expect("valid user id");

    assert!(info.is_exclusive_user_match(&claimed));
    assert!(!info.is_exclusive_user_match(&other));
}

#[test]
fn a_non_exclusive_namespace_is_matched_but_not_claimed() {
    let mut namespaces = Namespaces::new();
    namespaces.users = vec![Namespace::new(false, "@watched_.*".to_owned())];

    let info = registration("watcher", "watcher", "watcher", namespaces.users);
    let watched = UserId::parse("@watched_bob:phantom.test").expect("valid user id");

    assert!(info.is_user_match(&watched));
    assert!(!info.is_exclusive_user_match(&watched));
}

#[test]
fn an_appservice_always_matches_its_own_sender() {
    // No namespace covers it: the sender is the appservice's own user whether
    // or not the registration says so.
    let info = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));
    let sender = UserId::parse("@irc_bot:phantom.test").expect("valid user id");

    assert!(info.is_exclusive_user_match(&sender));
    assert_eq!(
        info.sender_user(&server_name()).expect("valid localpart"),
        sender
    );
}

#[test]
fn distinct_registrations_do_not_collide() {
    let irc = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));
    let xmpp = registration("xmpp", "xmpp_bot", "xmpp", exclusive_users("@xmpp_.*"));

    check_collisions(&registered([irc]), &xmpp, &server_name()).expect("no collision");
}

#[test]
fn a_registration_may_replace_itself() {
    let old = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));
    let new = registration("irc", "irc_bot", "irc", exclusive_users("@irc2_.*"));

    check_collisions(&registered([old]), &new, &server_name()).expect("no collision with itself");
}

#[test]
fn a_shared_token_collides() {
    let irc = registration("irc", "irc_bot", "shared", exclusive_users("@irc_.*"));
    let xmpp = registration("xmpp", "xmpp_bot", "shared", exclusive_users("@xmpp_.*"));

    check_collisions(&registered([irc]), &xmpp, &server_name())
        .expect_err("the tokens are the same");
}

#[test]
fn a_sender_inside_another_exclusive_namespace_collides() {
    let irc = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));

    // Sends as a user the registered appservice claims for itself.
    let squatter = registration("squatter", "irc_squatter", "squatter", vec![]);
    check_collisions(&registered([irc.clone()]), &squatter, &server_name())
        .expect_err("the sender is claimed by irc");

    // And the other way round: claims the user the registered one sends as.
    let claimer = registration("claimer", "claimer", "claimer", exclusive_users("@irc_bot"));
    check_collisions(&registered([irc]), &claimer, &server_name())
        .expect_err("irc's sender is claimed");
}

#[test]
fn the_same_exclusive_pattern_twice_collides() {
    let irc = registration("irc", "irc_bot", "irc", exclusive_users("@irc_.*"));
    let twin = registration("twin", "twin_bot", "twin", exclusive_users("@irc_.*"));

    check_collisions(&registered([irc]), &twin, &server_name())
        .expect_err("the pattern is already claimed");
}

#[test]
fn only_the_same_kind_of_namespace_overlaps() {
    let mut users = Namespaces::new();
    users.users = vec![Namespace::new(true, "#shared_.*".to_owned())];

    let mut aliases = Namespaces::new();
    aliases.aliases = vec![Namespace::new(true, "#shared_.*".to_owned())];

    assert_eq!(exclusive_overlap(&users, &aliases), None);
    assert_eq!(exclusive_overlap(&users, &users), Some("#shared_.*"));
}

#[test]
fn a_non_exclusive_pattern_may_be_shared() {
    let mut lhs = Namespaces::new();
    lhs.users = vec![Namespace::new(false, "@watched_.*".to_owned())];

    assert_eq!(exclusive_overlap(&lhs, &lhs), None);
}
