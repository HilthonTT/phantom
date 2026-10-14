use futures::StreamExt;
use ruma::{OwnedUserId, UserId, owned_user_id};

use crate::runtime::testing;

fn local(localpart: &str) -> OwnedUserId {
    UserId::parse_with_server_name(localpart, testing::SERVER_NAME.try_into().unwrap()).unwrap()
}

async fn listed(services: &crate::Services) -> Vec<OwnedUserId> {
    services
        .users
        .list_local_users()
        .map(ToOwned::to_owned)
        .collect()
        .await
}

#[tokio::test]
async fn deactivation_is_its_own_marker() {
    let services = testing::build("");
    let users = &services.users;
    let alice = local("alice");
    let admin = local("admin");

    users.create(&alice, Some("hunter2")).unwrap();
    assert!(!users.is_deactivated(&alice).await.unwrap());

    users
        .deactivate_account(&alice, Some(&admin))
        .await
        .unwrap();

    assert!(users.is_deactivated(&alice).await.unwrap());
    assert!(!users.is_active(&alice).await);
    assert!(
        !users.is_guest(&alice).await,
        "a deactivated account is no guest"
    );
    assert!(users.password_hash(&alice).await.unwrap().is_empty());
    assert_eq!(users.deactivation(&alice).await.unwrap().by, Some(admin));
}

#[tokio::test]
async fn a_new_password_does_not_reactivate() {
    let services = testing::build("");
    let users = &services.users;
    let alice = local("alice");

    users.create(&alice, Some("hunter2")).unwrap();
    users.deactivate_account(&alice, None).await.unwrap();
    users.set_password(&alice, Some("another")).unwrap();

    assert!(
        users.is_deactivated(&alice).await.unwrap(),
        "only reactivation lifts the marker"
    );

    users.reactivate_account(&alice, "another").await.unwrap();

    assert!(!users.is_deactivated(&alice).await.unwrap());
    assert!(!users.password_hash(&alice).await.unwrap().is_empty());
    assert!(
        users.reactivate_account(&alice, "again").await.is_err(),
        "an active account cannot be reactivated"
    );
}

#[tokio::test]
async fn passwordless_accounts_are_not_deactivated() {
    let services = testing::build("");
    let users = &services.users;
    let guest = local("k3x9q0z7ab");
    let alice = local("alice");
    let remote = owned_user_id!("@bob:remote.test");
    let server_user = services.server_state.server_user.clone();

    users.create(&guest, None).unwrap();
    users.create(&alice, Some("hunter2")).unwrap();
    users.create(&remote, None).unwrap();
    users.create(&server_user, None).unwrap();

    assert!(!users.is_deactivated(&guest).await.unwrap());
    assert!(users.is_active_local(&guest).await);
    assert!(users.is_guest(&guest).await);
    assert!(!users.is_guest(&alice).await);
    assert!(
        !users.is_guest(&remote).await,
        "a remote user is no local guest"
    );
    assert!(!users.is_guest(&server_user).await);

    let mut local_users = listed(&services).await;
    local_users.sort();
    assert_eq!(
        local_users,
        [alice.clone(), guest.clone()],
        "local accounts, guests included; no remote user and no server user"
    );

    users.deactivate_account(&guest, None).await.unwrap();
    assert_eq!(listed(&services).await, [alice]);
}

#[tokio::test]
async fn an_unknown_user_is_not_found() {
    let services = testing::build("");

    assert!(
        services
            .users
            .is_deactivated(&local("nobody"))
            .await
            .is_err()
    );
    assert!(!services.users.is_active(&local("nobody")).await);
}

#[tokio::test]
async fn the_emergency_logout_does_not_deactivate() {
    let services = testing::build("");
    let users = &services.users;
    let server_user = services.server_state.server_user.clone();

    users.create(&server_user, Some("emergency")).unwrap();
    assert!(
        users.is_active_local(&server_user).await,
        "with the emergency password, someone can act as the server user"
    );

    users.revoke_credentials(&server_user).await.unwrap();

    assert!(users.password_hash(&server_user).await.unwrap().is_empty());
    assert!(!users.is_deactivated(&server_user).await.unwrap());
    assert!(
        !users.is_active_local(&server_user).await,
        "without it the server user is no member to notify or auto-join"
    );
}
