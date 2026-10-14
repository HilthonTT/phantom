use std::collections::HashSet;

use regex::RegexSet;

use super::{STEPS, matched_patterns};

#[test]
fn step_markers_are_unique() {
    let mut seen = HashSet::new();

    for step in STEPS {
        assert!(
            seen.insert(step.marker),
            "duplicate migration marker {}",
            step.marker
        );
    }
}

#[test]
fn matching_patterns_are_listed_in_order() {
    let set = RegexSet::new(["^adm", "bad", "min$"]).expect("valid patterns");

    assert_eq!(matched_patterns(&set, "admin"), "^adm, min$");
}

#[test]
fn no_match_lists_nothing() {
    let set = RegexSet::new(["^adm"]).expect("valid pattern");

    assert!(matched_patterns(&set, "alice").is_empty());
}

mod deactivated {
    use ruma::{
        OwnedUserId, UserId,
        api::appservice::{Namespace, Namespaces, Registration, RegistrationInit},
    };

    use super::super::deactivated::mark;
    use crate::runtime::testing;

    fn local(localpart: &str) -> OwnedUserId {
        UserId::parse_with_server_name(localpart, testing::SERVER_NAME.try_into().unwrap()).unwrap()
    }

    fn bridge() -> Registration {
        let mut namespaces = Namespaces::new();
        namespaces.users = vec![Namespace::new(true, "@bridge_.*".to_owned())];

        RegistrationInit {
            id: "bridge".to_owned(),
            url: None,
            as_token: "as_token".to_owned(),
            hs_token: "hs_token".to_owned(),
            sender_localpart: "bridgebot".to_owned(),
            namespaces,
            rate_limited: None,
            protocols: None,
        }
        .into()
    }

    #[tokio::test]
    async fn only_accounts_bearing_every_trace_of_deactivation_are_marked() {
        let services = testing::build("");
        let users = &services.users;

        // Deactivated before the marker: no password, device, name or room.
        let gone = local("gone");
        users.create(&gone, None).unwrap();

        // A guest: no password, but registration gave it a display name.
        let guest = local("k3x9q0z7ab");
        users.create(&guest, None).unwrap();
        services
            .profile
            .set_displayname(&guest, Some("k3x9q0z7ab".to_owned()));

        // A puppet in a bridge's namespace, and the bridge's own sender.
        services
            .appservice
            .register_appservice(&bridge())
            .await
            .unwrap();
        let puppet = local("bridge_alice");
        users.create(&puppet, None).unwrap();
        let bot = local("bridgebot");
        users.create(&bot, None).unwrap();

        let alice = local("alice");
        users.create(&alice, Some("hunter2")).unwrap();

        let server_user = services.server_state.server_user.clone();
        users.create(&server_user, None).unwrap();

        assert!(mark(&services).await.unwrap(), "the step finishes");

        assert!(users.is_deactivated(&gone).await.unwrap());
        for user_id in [&guest, &puppet, &bot, &alice, &server_user] {
            assert!(
                !users.is_deactivated(user_id).await.unwrap(),
                "{user_id} was marked"
            );
        }
    }
}
