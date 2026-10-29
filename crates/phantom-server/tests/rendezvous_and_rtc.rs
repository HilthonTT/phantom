//! End-to-end: the rendezvous sessions QR-code login passes its messages
//! through, in both protocols, and the MatrixRTC transports group calls are
//! pointed at.

#![cfg(unix)]

mod support;

use reqwest::{
    Method, StatusCode,
    header::{CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH},
};
use serde_json::{Value, json};
use support::{Account, Server, client, free_port, string};

const SERVER_NAME: &str = "rendezvous.test";
const MSC4388: &str = "/_matrix/client/unstable/io.element.msc4388/rendezvous";
const MSC4108: &str = "/_matrix/client/unstable/org.matrix.msc4108/rendezvous";
const LIVEKIT: &str = "https://livekit-jwt.rendezvous.test";

#[tokio::test(flavor = "multi_thread")]
async fn qr_login_sessions_and_rtc_transports_are_served() {
    let port = free_port();
    let mut server = Server::start(
        SERVER_NAME,
        &[port],
        &format!(
            r#"
            allow_federation = false
            well_known_client = "http://127.0.0.1:{port}"

            [[global.rtc_transports]]
            type = "livekit"
            livekit_service_url = "{LIVEKIT}"
            "#
        ),
    )
    .await;

    let ada = Account::register(&server, "ada").await;
    let anonymous = client();

    // Both features are advertised.
    let versions: Value = anonymous
        .get(format!("{}/_matrix/client/versions", server.url))
        .send()
        .await
        .expect("versions")
        .text()
        .await
        .map(|text| parse(&text))
        .expect("a body");
    for feature in [
        "org.matrix.msc4108",
        "io.element.msc4388",
        "org.matrix.msc4143",
    ] {
        assert_eq!(
            versions["unstable_features"][feature], true,
            "{feature}: {versions}"
        );
    }

    // MatrixRTC: the transport, from its endpoint and from .well-known.
    let transports = ada
        .ok(
            Method::GET,
            "/_matrix/client/unstable/org.matrix.msc4143/rtc/transports",
            None,
        )
        .await;
    let livekit = json!([{"type": "livekit", "livekit_service_url": LIVEKIT}]);
    assert_eq!(transports["rtc_transports"], livekit, "{transports}");

    let well_known: Value = anonymous
        .get(format!("{}/.well-known/matrix/client", server.url))
        .send()
        .await
        .expect("well-known")
        .text()
        .await
        .map(|text| parse(&text))
        .expect("a body");
    assert_eq!(
        well_known["org.matrix.msc4143.rtc_foci"], livekit,
        "{well_known}"
    );

    // MSC4388: only a signed-in device may create a session.
    let discover = |token: Option<&str>| {
        let mut request = anonymous.get(format!("{}{MSC4388}", server.url));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        async move {
            request
                .send()
                .await
                .expect("discover")
                .text()
                .await
                .map(|text| parse(&text))
                .expect("a body")
        }
    };
    assert_eq!(discover(None).await["create_available"], false);
    assert_eq!(discover(Some(&ada.token)).await["create_available"], true);

    let refused = anonymous
        .post(format!("{}{MSC4388}", server.url))
        .header(CONTENT_TYPE, "application/json")
        .body(json!({"data": "hello"}).to_string())
        .send()
        .await
        .expect("create");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let created = ada
        .ok(Method::POST, MSC4388, Some(json!({"data": "hello"})))
        .await;
    let session = format!("{}{MSC4388}/{}", server.url, string(&created["id"]));
    let first = string(&created["sequence_token"]);
    assert!(created["expires_in_ms"].as_u64() > Some(0), "{created}");

    // The other device needs no token to read and answer.
    let read: Value = anonymous
        .get(&session)
        .send()
        .await
        .expect("get")
        .text()
        .await
        .map(|text| parse(&text))
        .expect("a body");
    assert_eq!(read["data"], "hello");
    assert_eq!(read["sequence_token"], first.as_str());

    let put = |token: &str, data: &str| {
        anonymous
            .put(&session)
            .header(CONTENT_TYPE, "application/json")
            .body(json!({"sequence_token": token, "data": data}).to_string())
            .send()
    };
    let answered = put(&first, "hi back").await.expect("put");
    assert_eq!(answered.status(), StatusCode::OK);
    let second = string(
        &answered
            .text()
            .await
            .map(|text| parse(&text))
            .expect("a body")["sequence_token"],
    );
    assert_ne!(first, second);

    // A write from a stale token is a conflict, not a lost update.
    let stale = put(&first, "something else").await.expect("put");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let error: Value = stale.text().await.map(|text| parse(&text)).expect("a body");
    assert_eq!(
        error["errcode"], "IO_ELEMENT_MSC4388_CONCURRENT_WRITE",
        "{error}"
    );

    let deleted = anonymous.delete(&session).send().await.expect("delete");
    assert_eq!(deleted.status(), StatusCode::OK);
    let gone = anonymous.get(&session).send().await.expect("get");
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    // MSC4108: raw text, versioned by ETag, at a URL the server hands out.
    let refused = anonymous
        .post(format!("{}{MSC4108}", server.url))
        .header(CONTENT_TYPE, "text/plain")
        .body("hello")
        .send()
        .await
        .expect("create");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let created = anonymous
        .post(format!("{}{MSC4108}", server.url))
        .bearer_auth(&ada.token)
        .header(CONTENT_TYPE, "text/plain")
        .body("hello")
        .send()
        .await
        .expect("create");
    assert_eq!(created.status(), StatusCode::OK);
    let first = created.headers()[ETAG].clone();
    let url = string(
        &created
            .text()
            .await
            .map(|text| parse(&text))
            .expect("a body")["url"],
    );
    assert!(
        url.starts_with(&format!("{}{MSC4108}/", server.url)),
        "{url}"
    );

    let read = anonymous.get(&url).send().await.expect("get");
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(read.headers()[ETAG], first);
    assert_eq!(read.text().await.expect("text"), "hello");

    let unchanged = anonymous
        .get(&url)
        .header(IF_NONE_MATCH, &first)
        .send()
        .await
        .expect("get");
    assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);

    let answered = anonymous
        .put(&url)
        .header(IF_MATCH, &first)
        .header(CONTENT_TYPE, "text/plain")
        .body("hi back")
        .send()
        .await
        .expect("put");
    assert_eq!(answered.status(), StatusCode::ACCEPTED);
    assert_ne!(answered.headers()[ETAG], first);

    let stale = anonymous
        .put(&url)
        .header(IF_MATCH, &first)
        .header(CONTENT_TYPE, "text/plain")
        .body("something else")
        .send()
        .await
        .expect("put");
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);

    let read = anonymous.get(&url).send().await.expect("get");
    assert_eq!(read.text().await.expect("text"), "hi back");

    let deleted = anonymous.delete(&url).send().await.expect("delete");
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let gone = anonymous.get(&url).send().await.expect("get");
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    server.stop_cleanly();
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"))
}
