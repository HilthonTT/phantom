//! End-to-end smoke test: one real server, driven the way a client and an
//! admin would, from a fresh database to a clean shutdown.
//!
//! The unit tests exercise the pieces; this is what catches the failures that
//! only exist once they are assembled, such as a route registered twice
//! (axum panics at startup) or a service whose `Dep` nothing builds.

#![cfg(unix)]

mod support;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use support::{Account, Server, body, encode, free_port, string};

const SERVER_NAME: &str = "smoke.test";

#[tokio::test(flavor = "multi_thread")]
async fn a_fresh_server_registers_chats_and_administers() {
    // Federation is off: this test is about one server, and nothing here
    // should reach past the loopback interface.
    let mut server = Server::start(SERVER_NAME, &[free_port()], "allow_federation = false").await;

    // The first account on a fresh database becomes its admin; the second
    // does not.
    let mut ada = Account::register(&server, "ada").await;
    let grace = Account::register(&server, "grace").await;
    assert_eq!(ada.user_id, format!("@ada:{SERVER_NAME}"));

    // A room, a message, and the message coming back through /sync.
    let room_id = ada
        .create_room(json!({"name": "Smoke", "preset": "private_chat"}))
        .await;
    let event_id = ada.say(&room_id, "hello from the smoke test").await;
    let seen = ada
        .await_event(&room_id, |event| event["event_id"] == event_id.as_str())
        .await;
    assert_eq!(body(&seen), "hello from the smoke test");

    // The admin room exists under its alias, and ada was joined to it when
    // she registered. A command sent there is answered by the server user.
    let alias = format!("#admins:{SERVER_NAME}");
    let resolved = ada
        .ok(
            Method::GET,
            &format!("/_matrix/client/v3/directory/room/{}", encode(&alias)),
            None,
        )
        .await;
    let admin_room = string(&resolved["room_id"]);

    ada.say(&admin_room, "!admin users list").await;
    let server_user = format!("@phantom:{SERVER_NAME}");
    let reply = ada
        .await_event(&admin_room, |event| {
            event["sender"] == server_user.as_str() && body(event).contains("@grace:")
        })
        .await;
    assert!(
        body(&reply).contains(&ada.user_id),
        "the user list names every account: {reply}"
    );

    // The admin API answers the admin, with the same accounts.
    let whoami = ada.ok(Method::GET, "/_phantom/admin/v1/whoami", None).await;
    assert_eq!(whoami["user_id"], ada.user_id.as_str());
    assert_eq!(whoami["server_name"], SERVER_NAME);

    let users = ada.ok(Method::GET, "/_phantom/admin/v1/users", None).await;
    let admin_flag = |user_id: &str| {
        users
            .as_array()
            .and_then(|users| users.iter().find(|user| user["user_id"] == user_id))
            .map(|user| user["admin"].clone())
    };
    assert_eq!(
        admin_flag(&ada.user_id),
        Some(Value::Bool(true)),
        "users: {users}"
    );
    assert_eq!(
        admin_flag(&grace.user_id),
        Some(Value::Bool(false)),
        "users: {users}"
    );

    let rooms = ada.ok(Method::GET, "/_phantom/admin/v1/rooms", None).await;
    assert!(rooms.to_string().contains(&room_id), "rooms: {rooms}");

    ada.ok(Method::GET, "/_phantom/admin/v1/stats", None).await;
    ada.ok(Method::GET, "/_phantom/admin/v1/services", None)
        .await;

    // ...and nobody else.
    let (status, refused) = grace
        .request(Method::GET, "/_phantom/admin/v1/users", None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a non-admin got: {refused}");

    server.stop_cleanly();
}

/// The features that only show their worth together: a message that notifies
/// lands in `/notifications`, an admin action lands in the audit log and
/// sticks through a password reset, the runtime block list and the route
/// table answer, and `/metrics` is served only to the scraper holding the
/// token.
#[tokio::test(flavor = "multi_thread")]
async fn notifications_audit_and_metrics_work_end_to_end() {
    let mut server = Server::start(
        SERVER_NAME,
        &[free_port()],
        r#"
        allow_federation = false
        allow_metrics = true
        metrics_token = "scrape-token"
        "#,
    )
    .await;

    let mut ada = Account::register(&server, "ada").await;
    let grace = Account::register(&server, "grace").await;

    // Two members make it a one-to-one room, where every message notifies
    // under the default push rules.
    let room_id = ada
        .create_room(json!({"preset": "private_chat", "invite": [grace.user_id]}))
        .await;
    grace
        .ok(
            Method::POST,
            &format!("/_matrix/client/v3/join/{}", encode(&room_id)),
            Some(json!({})),
        )
        .await;
    let event_id = ada.say(&room_id, "are you there, grace?").await;

    let notified = poll(|| async {
        let page = grace
            .ok(Method::GET, "/_matrix/client/v3/notifications", None)
            .await;
        page["notifications"].as_array().and_then(|list| {
            list.iter()
                .find(|n| n["event"]["event_id"] == event_id.as_str())
                .cloned()
        })
    })
    .await;
    assert_eq!(notified["room_id"], room_id.as_str());
    assert_eq!(notified["read"], false, "not read yet: {notified}");

    // The route table lists what is actually served.
    let routes = ada.ok(Method::GET, "/_phantom/admin/v1/routes", None).await;
    assert!(
        routes
            .to_string()
            .contains("/_matrix/client/v3/notifications"),
        "routes: {routes}"
    );

    // A deactivation is audited, refuses the login, and a password reset does
    // not undo it.
    ada.ok(
        Method::POST,
        &format!(
            "/_phantom/admin/v1/users/{}/deactivate",
            encode(&grace.user_id)
        ),
        Some(json!({"erase": false})),
    )
    .await;

    let login = support::client()
        .post(format!("{}/_matrix/client/v3/login", server.url))
        .body(
            json!({
                "type": "m.login.password",
                "identifier": {"type": "m.id.user", "user": "grace"},
                "password": support::PASSWORD,
            })
            .to_string(),
        )
        .send()
        .await
        .expect("login");
    assert_eq!(login.status(), StatusCode::FORBIDDEN);
    let refused: Value =
        serde_json::from_str(&login.text().await.expect("a body")).expect("a JSON error");
    assert_eq!(refused["errcode"], "M_USER_DEACTIVATED", "{refused}");

    let (status, reset) = ada
        .request(
            Method::PUT,
            &format!(
                "/_phantom/admin/v1/users/{}/password",
                encode(&grace.user_id)
            ),
            Some(json!({"password": "a new one"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "reset: {reset}");

    let audit = ada.ok(Method::GET, "/_phantom/admin/v1/audit", None).await;
    let deactivation = audit["records"]
        .as_array()
        .and_then(|records| {
            records
                .iter()
                .find(|r| string(&r["action"]).contains("deactivate"))
        })
        .unwrap_or_else(|| panic!("no deactivation in the audit log: {audit}"));
    assert_eq!(deactivation["admin"], ada.user_id.as_str());
    assert_eq!(deactivation["ok"], true);
    assert!(
        string(&deactivation["target"]).contains(&grace.user_id),
        "{deactivation}"
    );

    // The runtime block list takes a server and shows it.
    ada.ok(
        Method::POST,
        "/_phantom/admin/v1/federation/blocks",
        Some(json!({"server": "evil.example", "reason": "smoke"})),
    )
    .await;
    let blocks = ada
        .ok(Method::GET, "/_phantom/admin/v1/federation/blocks", None)
        .await;
    assert!(blocks.to_string().contains("evil"), "blocks: {blocks}");

    // /metrics answers the scraper with the token, and nobody else.
    let metrics_url = format!("{}/metrics", server.url);
    let anonymous = support::client()
        .get(&metrics_url)
        .send()
        .await
        .expect("metrics");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let scraped = support::client()
        .get(&metrics_url)
        .bearer_auth("scrape-token")
        .send()
        .await
        .expect("metrics");
    assert_eq!(scraped.status(), StatusCode::OK);
    let exposition = scraped.text().await.expect("the exposition");
    assert!(
        exposition.contains("phantom_build_info"),
        "exposition: {exposition}"
    );
    assert!(
        exposition.contains("/_matrix/client/v3/notifications"),
        "requests are counted by route template: {exposition}"
    );

    ada.say(&room_id, "goodbye").await;
    server.stop_cleanly();
}

/// Polls `check` until it yields a value, for what the server does in the
/// background after answering.
async fn poll<F, Fut, T>(check: F) -> T
where
    F: Fn() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    for _ in 0..100 {
        if let Some(found) = check().await {
            return found;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    panic!("gave up waiting");
}
