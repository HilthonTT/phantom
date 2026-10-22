use futures::StreamExt;
use ruma::{
    OneTimeKeyAlgorithm, OwnedDeviceId, OwnedOneTimeKeyId, OwnedUserId, UserId,
    encryption::OneTimeKey, owned_user_id, serde::Raw,
};

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

fn curve25519_key(key_id: &str, key: &str) -> (OwnedOneTimeKeyId, Raw<OneTimeKey>) {
    let key = serde_json::json!({ "key": key, "fallback": true }).to_string();
    let key = serde_json::from_str(&key).unwrap();
    (key_id.try_into().unwrap(), key)
}

#[tokio::test]
async fn a_fallback_key_is_handed_out_until_replaced() {
    let services = testing::build("");
    let users = &services.users;
    let alice = local("alice");
    let device: OwnedDeviceId = "PHONE".into();
    let algorithm = OneTimeKeyAlgorithm::SignedCurve25519;

    users.create(&alice, Some("hunter2")).unwrap();
    users
        .create_device(&alice, &device, "token", None, None)
        .await
        .unwrap();

    assert!(
        users
            .unused_fallback_key_types(&alice, &device)
            .await
            .is_empty()
    );
    assert!(
        users
            .take_fallback_key(&alice, &device, &algorithm)
            .await
            .is_err()
    );

    let (first_id, first) = curve25519_key("signed_curve25519:AAAA", "first");
    users
        .set_fallback_key(&alice, &device, &first_id, &first)
        .await
        .unwrap();
    assert_eq!(
        users.unused_fallback_key_types(&alice, &device).await,
        std::slice::from_ref(&algorithm)
    );

    for _ in 0..2 {
        let (key_id, _) = users
            .take_fallback_key(&alice, &device, &algorithm)
            .await
            .unwrap();
        assert_eq!(key_id, first_id, "a used fallback key is not removed");
    }
    assert!(
        users
            .unused_fallback_key_types(&alice, &device)
            .await
            .is_empty()
    );

    users
        .set_fallback_key(&alice, &device, &first_id, &first)
        .await
        .unwrap();
    assert!(
        users
            .unused_fallback_key_types(&alice, &device)
            .await
            .is_empty(),
        "uploading the same key again does not make it unused"
    );

    let (second_id, second) = curve25519_key("signed_curve25519:BBBB", "second");
    users
        .set_fallback_key(&alice, &device, &second_id, &second)
        .await
        .unwrap();
    assert_eq!(
        users.unused_fallback_key_types(&alice, &device).await,
        std::slice::from_ref(&algorithm)
    );

    let (key_id, _) = users
        .take_fallback_key(&alice, &device, &algorithm)
        .await
        .unwrap();
    assert_eq!(key_id, second_id);

    users.remove_device(&alice, &device).await;
    assert!(
        users
            .take_fallback_key(&alice, &device, &algorithm)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_fallback_key_needs_a_device() {
    let services = testing::build("");
    let users = &services.users;
    let alice = local("alice");
    let device: OwnedDeviceId = "GONE".into();

    users.create(&alice, Some("hunter2")).unwrap();

    let (key_id, key) = curve25519_key("signed_curve25519:AAAA", "key");
    assert!(
        users
            .set_fallback_key(&alice, &device, &key_id, &key)
            .await
            .is_err()
    );
}
