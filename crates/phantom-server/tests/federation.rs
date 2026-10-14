//! End-to-end federation test: two real servers on loopback, a room created on
//! one and joined from the other, and messages flowing both ways.
//!
//! Each server is named by an IP literal with its federation port
//! (`127.0.0.1:<port>`), which server-name resolution takes as-is, so no DNS
//! or `.well-known` is involved. Federation always speaks HTTPS, so the
//! federation port serves the self-signed certificate in `tests/fixtures`
//! (valid for 127.0.0.1 until 2126) and both servers accept it through
//! `allow_invalid_tls_certificates`. Clients stay on a plain port beside it,
//! via `[global.tls] ports`.

#![cfg(unix)]

mod support;

use std::path::Path;

use reqwest::Method;
use serde_json::json;
use support::{Account, Server, body, encode, free_port};

async fn start_federating() -> Server {
    let client_port = free_port();
    let federation_port = free_port();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    // The default denylist covers loopback, which is exactly where the peer
    // is. Trusted key servers are already off, so keys come from the peer.
    let extra = format!(
        r#"allow_federation = true
ip_range_denylist = []
allow_invalid_tls_certificates = true

[global.tls]
certs = "{certs}"
key = "{key}"
ports = [{federation_port}]
"#,
        certs = fixtures.join("loopback.crt").display(),
        key = fixtures.join("loopback.key").display(),
    );

    Server::start(
        &format!("127.0.0.1:{federation_port}"),
        &[client_port, federation_port],
        &extra,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_user_on_another_server_joins_and_both_talk() {
    let (mut a, mut b) = tokio::join!(start_federating(), start_federating());

    let mut alice = Account::register(&a, "alice").await;
    let mut bob = Account::register(&b, "bob").await;

    let room_id = alice
        .create_room(json!({"name": "Across", "preset": "public_chat"}))
        .await;

    // Joining by room ID needs a server to join through; alice's is the only
    // one in the room.
    let join = format!(
        "/_matrix/client/v3/join/{}?server_name={}",
        encode(&room_id),
        encode(&a.server_name)
    );
    bob.ok(Method::POST, &join, Some(json!({}))).await;

    alice.say(&room_id, "hello from A").await;
    let seen = bob
        .await_event(&room_id, |event| body(event) == "hello from A")
        .await;
    assert_eq!(seen["sender"], alice.user_id.as_str());

    bob.say(&room_id, "hello from B").await;
    let seen = alice
        .await_event(&room_id, |event| body(event) == "hello from B")
        .await;
    assert_eq!(seen["sender"], bob.user_id.as_str());

    a.stop_cleanly();
    b.stop_cleanly();
}
