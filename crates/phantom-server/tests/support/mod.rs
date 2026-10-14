//! A scratch phantom-server for the end-to-end tests: the built binary, run on
//! its own database in a temporary directory, on free loopback ports, and
//! driven over HTTP the way a client would.
//!
//! The binary is spawned rather than run in-process because what these tests
//! are for is the startup a real deployment goes through: config parsing,
//! service wiring and route registration all happen in `main`, and an axum
//! "Overlapping method route" panic or a missing service `Dep` only shows up
//! there. A process also gives the shutdown a real signal to honour.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the harness"
)]

use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;

pub const REGISTRATION_TOKEN: &str = "smoke-test-token";
pub const PASSWORD: &str = "correct horse battery staple";

/// How long a fresh server gets to start answering. A debug build on a slow CI
/// runner opens RocksDB and runs the migrations in a few seconds; this is
/// generous so the test fails on a hang rather than a slow machine.
const START_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the server gets to exit after SIGTERM before it counts as hung.
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a sync loop waits for an expected event.
pub const EVENT_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Server {
    /// The base URL clients use, which is the plain HTTP port.
    pub url: String,
    pub server_name: String,

    child: Option<Child>,
    log: PathBuf,

    // Held so the directory outlives the process; removed on drop.
    _dir: TempDir,
}

/// Picks a loopback port nothing is listening on, for a server to bind a
/// few seconds later, once its database is open.
///
/// The port comes from below the kernel's ephemeral range (32768 and up on
/// Linux), which outgoing connections draw from: the tests' own requests
/// open those by the hundred while a server starts, and one could otherwise
/// take the port first. Ports handed out are remembered, so two calls never
/// return the same one, even before either is bound.
pub fn free_port() -> u16 {
    use std::{
        collections::BTreeSet,
        hash::{BuildHasher, RandomState},
        sync::Mutex,
    };

    const LOW: u16 = 20_000;
    const SPAN: u64 = 12_000;

    static TAKEN: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

    let mut taken = TAKEN.lock().expect("the port set is not poisoned");
    let random = RandomState::new();

    for attempt in 0_u64..10_000 {
        let offset = u16::try_from(random.hash_one(attempt) % SPAN).expect("below the span");
        let port = LOW + offset;

        if !taken.contains(&port) && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            taken.insert(port);
            return port;
        }
    }

    panic!(
        "no free loopback port between {LOW} and {}",
        u64::from(LOW) + SPAN
    );
}

impl Server {
    /// Starts a server named `server_name`, listening on `ports` (the first
    /// is the one clients use), with `extra` appended to its `[global]`
    /// section. Further sections may follow in `extra`.
    pub async fn start(server_name: &str, ports: &[u16], extra: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("phantom-e2e-")
            .tempdir()
            .expect("create a scratch directory");

        let ports = ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");

        // Everything a fresh server would otherwise reach the internet for is
        // turned off: no trusted key servers, no update check, no startup
        // federation burst.
        let config = format!(
            r#"[global]
server_name = "{server_name}"
address = ["127.0.0.1"]
port = [{ports}]
database_path = "{db}"
database_backup_path = "{backups}"
allow_registration = true
registration_token = "{REGISTRATION_TOKEN}"
trusted_servers = []
allow_check_for_updates = false
startup_netburst = false
log = "info"
log_colors = false
{extra}
"#,
            db = dir.path().join("db").display(),
            backups = dir.path().join("backups").display(),
        );

        let config_path = dir.path().join("phantom.toml");
        fs::write(&config_path, config).expect("write the config");

        // stdout and stderr go to one file rather than a pipe: nobody reads a
        // pipe while the test runs, and a full one would block the server's
        // logging and so the server.
        let log = dir.path().join("server.log");
        let out = fs::File::create(&log).expect("create the log file");
        let err = out.try_clone().expect("clone the log file handle");

        let child = Command::new(env!("CARGO_BIN_EXE_phantom-server"))
            .arg("--config")
            .arg(&config_path)
            .env("RUST_BACKTRACE", "1")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
            .expect("spawn phantom-server");

        let url = format!(
            "http://127.0.0.1:{}",
            ports.split(',').next().unwrap_or_default()
        );
        let mut server = Self {
            url,
            server_name: server_name.to_owned(),
            child: Some(child),
            log,
            _dir: dir,
        };

        server.wait_until_ready().await;
        server
    }

    async fn wait_until_ready(&mut self) {
        let client = client();
        let versions = format!("{}/_matrix/client/versions", self.url);
        let deadline = Instant::now() + START_TIMEOUT;

        loop {
            if let Some(status) = self
                .child_mut()
                .try_wait()
                .expect("poll the server process")
            {
                panic!(
                    "phantom-server exited during startup ({status}):\n{}",
                    self.log()
                );
            }

            if let Ok(response) = client.get(&versions).send().await
                && response.status() == StatusCode::OK
            {
                return;
            }

            assert!(
                Instant::now() < deadline,
                "phantom-server did not answer /versions within {START_TIMEOUT:?}:\n{}",
                self.log()
            );

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("server is running")
    }

    /// Everything the server has logged so far.
    pub fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Stops the server with SIGTERM, as a service manager would, and returns
    /// its exit status and its whole log.
    pub fn stop(&mut self) -> (ExitStatus, String) {
        let mut child = self.child.take().expect("server is running");
        terminate(&child);

        let deadline = Instant::now() + STOP_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll the server process") {
                break status;
            }

            if Instant::now() >= deadline {
                child.kill().ok();
                child.wait().ok();
                panic!(
                    "phantom-server did not exit within {STOP_TIMEOUT:?} of SIGTERM:\n{}",
                    self.log()
                );
            }

            thread::sleep(Duration::from_millis(100));
        };

        (status, self.log())
    }

    /// Stops the server and fails the test unless it exited cleanly without a
    /// panic anywhere in its log, including on threads the process survived.
    pub fn stop_cleanly(&mut self) {
        let (status, log) = self.stop();

        assert!(
            !log.contains("panicked at"),
            "phantom-server panicked:\n{log}"
        );
        assert!(
            status.success(),
            "phantom-server exited with {status}:\n{log}"
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A test that failed half-way still must not leave a server behind.
        if let Some(mut child) = self.child.take() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

/// Sends SIGTERM through `kill(1)`, which every unix the tests run on has,
/// rather than `libc::kill`, which would need `unsafe` for one call.
fn terminate(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("run kill");

    assert!(status.success(), "kill -TERM {} failed", child.id());
}

/// The HTTP client every test uses. Certificates are not verified, because the
/// federation test's servers present a self-signed one.
pub fn client() -> Client {
    Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(90))
        .build()
        .expect("build the HTTP client")
}

/// A signed-in account on one of the servers.
pub struct Account {
    pub base: String,
    pub user_id: String,
    pub token: String,
    client: Client,
    txn: u64,
}

impl Account {
    /// Registers `username` through the registration-token UIAA flow.
    pub async fn register(server: &Server, username: &str) -> Self {
        let client = client();
        let url = format!("{}/_matrix/client/v3/register", server.url);
        let mut body = json!({
            "username": username,
            "password": PASSWORD,
            "initial_device_display_name": "e2e",
        });

        // The first request is answered 401 with the flows and the session
        // the second request completes.
        let response = client
            .post(&url)
            .body(body.to_string())
            .send()
            .await
            .expect("register");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "first /register"
        );
        let flow: Value = json_of(response).await;
        let stages = &flow["flows"][0]["stages"];
        assert_eq!(
            stages,
            &json!(["m.login.registration_token"]),
            "registration flow: {flow}"
        );

        body["auth"] = json!({
            "type": "m.login.registration_token",
            "token": REGISTRATION_TOKEN,
            "session": flow["session"],
        });
        let response = client
            .post(&url)
            .body(body.to_string())
            .send()
            .await
            .expect("register");
        let status = response.status();
        let done: Value = json_of(response).await;
        assert_eq!(status, StatusCode::OK, "second /register: {done}");

        Self {
            base: server.url.clone(),
            user_id: string(&done["user_id"]),
            token: string(&done["access_token"]),
            client,
            txn: 0,
        }
    }

    /// Makes an authenticated request and returns the status and JSON body.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method.clone(), format!("{}{path}", self.base))
            .bearer_auth(&self.token);

        if let Some(body) = body {
            request = request
                .header("Content-Type", "application/json")
                .body(body.to_string());
        }

        let response = request
            .send()
            .await
            .unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        let status = response.status();

        (status, json_of(response).await)
    }

    /// Like `request`, but fails the test on anything other than 200.
    pub async fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.request(method.clone(), path, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");

        value
    }

    pub async fn create_room(&self, body: Value) -> String {
        let created = self
            .ok(Method::POST, "/_matrix/client/v3/createRoom", Some(body))
            .await;

        string(&created["room_id"])
    }

    pub async fn say(&mut self, room_id: &str, text: &str) -> String {
        self.txn += 1;
        let path = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/e2e{}",
            encode(room_id),
            self.txn
        );
        let sent = self
            .ok(
                Method::PUT,
                &path,
                Some(json!({"msgtype": "m.text", "body": text})),
            )
            .await;

        string(&sent["event_id"])
    }

    /// Syncs until `room_id`'s timeline holds an event `matches` accepts, and
    /// returns that event. Starts from scratch, so an event that arrived before
    /// the call is found too.
    pub async fn await_event(&self, room_id: &str, matches: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + EVENT_TIMEOUT;
        let mut since: Option<String> = None;
        let mut seen = Vec::new();

        while Instant::now() < deadline {
            let path = match &since {
                None => "/_matrix/client/v3/sync?timeout=0".to_owned(),
                Some(since) => format!(
                    "/_matrix/client/v3/sync?timeout=5000&since={}",
                    encode(since)
                ),
            };
            let sync = self.ok(Method::GET, &path, None).await;

            let events = sync["rooms"]["join"][room_id]["timeline"]["events"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            if let Some(event) = events.iter().find(|event| matches(event)) {
                return event.clone();
            }

            seen.extend(events);
            since = sync["next_batch"].as_str().map(ToOwned::to_owned);
        }

        panic!(
            "{} saw no matching event in {room_id} within {EVENT_TIMEOUT:?}; the timeline \
             held:\n{}",
            self.user_id,
            serde_json::to_string_pretty(&seen).unwrap_or_default()
        );
    }
}

async fn json_of(response: reqwest::Response) -> Value {
    let text = response.text().await.expect("read the response body");
    if text.is_empty() {
        return Value::Null;
    }

    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

pub fn string(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected a string, got {value}"))
        .to_owned()
}

/// Percent-encodes a path segment or query value. Room IDs, aliases and sync
/// tokens use only a handful of reserved characters, so this needs no crate.
pub fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }

    out
}

/// The body of a message event, or "" for anything else.
pub fn body(event: &Value) -> &str {
    event["content"]["body"].as_str().unwrap_or_default()
}
