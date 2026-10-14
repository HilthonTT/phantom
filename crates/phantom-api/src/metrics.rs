//! Prometheus metrics: the `/metrics` scrape, and the layer that times every
//! request for it.
//!
//! Counters are kept where they happen, on `Server::metrics`; this module
//! adds what has to be read from the services at scrape time, and writes the
//! lot out. What costs a database scan (the account and room counts, the
//! sending queues) is cached for [`SLOW_TTL`], so a scraper polling every few
//! seconds does not turn into a load of its own.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

use axum::{
    extract::{MatchedPath, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
};
use futures::StreamExt;
use phantom_core::{
    Result,
    diagnostics::info,
    hash::sha256,
    runtime::{
        metrics::{
            SendClass,
            http::UNMATCHED,
            text::{CONTENT_TYPE, Kind, Text},
        },
        server::Server,
    },
    warn,
};
use phantom_service::{Services, net::sending::QueueDepths};
use tokio::task::spawn_blocking;

use crate::router::State as RouterState;

/// How long the figures that need a database scan are reused for.
const SLOW_TTL: Duration = Duration::from_secs(60);

/// Adds `/metrics` to `router` for the main listeners, where a scrape must
/// always carry `metrics_token`.
pub fn register(router: axum::Router<RouterState>) -> axum::Router<RouterState> {
    router.route("/metrics", get(metrics_main))
}

/// Adds `/metrics` to `router` for the `metrics_address` listener, where the
/// token is asked for only when one is set.
pub fn register_own(router: axum::Router<RouterState>) -> axum::Router<RouterState> {
    router.route("/metrics", get(metrics_own))
}

/// Times a request and counts it by the route template it matched. Added
/// with `Router::layer`, which wraps each route, so the matched path is
/// already known when this runs.
pub async fn track(State(server): State<Arc<Server>>, request: Request, next: Next) -> Response {
    let route = request.extensions().get::<MatchedPath>().cloned();
    let method = request.method().clone();
    let metrics = &server.metrics;

    let _in_flight = InFlight::enter(&metrics.requests_handle_active);
    let started = Instant::now();
    let response = next.run(request).await;

    let route = route.as_ref().map_or(UNMATCHED, MatchedPath::as_str);
    metrics.http.observe(
        route,
        method.as_str(),
        response.status().as_u16(),
        started.elapsed(),
    );
    metrics
        .requests_handle_finished
        .fetch_add(1, Ordering::Relaxed);

    response
}

/// Counts a request in flight until it is answered or dropped, as a client
/// that hangs up drops it.
struct InFlight<'a>(&'a AtomicU32);

impl<'a> InFlight<'a> {
    fn enter(count: &'a AtomicU32) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        Self(count)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// # `GET /metrics` on the main listeners
async fn metrics_main(State(services): State<RouterState>, headers: HeaderMap) -> Response {
    metrics(&services, &headers, true).await
}

/// # `GET /metrics` on the `metrics_address` listener
async fn metrics_own(State(services): State<RouterState>, headers: HeaderMap) -> Response {
    metrics(&services, &headers, false).await
}

/// Every metric, in the Prometheus text format.
///
/// Which listener the route sits on is fixed at startup, while the config
/// can be reloaded since, so the listener decides whether a token is needed:
/// on the main listeners one always is, and with none configured any longer
/// the scrape is refused rather than served to everyone.
async fn metrics(services: &Services, headers: &HeaderMap, token_required: bool) -> Response {
    let logging = &services.server.config.logging;

    // Routed at startup; a reload that turns metrics off since closes it here.
    if !logging.allow_metrics {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Some(refusal) = refusal(headers, logging.metrics_token.as_deref(), token_required) {
        return refusal;
    }

    let mut text = Text::new();
    encode(services, &mut text).await;

    ([(header::CONTENT_TYPE, CONTENT_TYPE)], text.finish()).into_response()
}

/// Why a scrape is refused, if it is: a token is configured and the request
/// lacks it, or one is required and none is configured.
fn refusal(headers: &HeaderMap, token: Option<&str>, required: bool) -> Option<Response> {
    match token.filter(|token| !token.is_empty()) {
        Some(token) if authorized(headers, token) => None,
        Some(_) => Some(
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Bearer")],
            )
                .into_response(),
        ),
        None if required => Some(StatusCode::FORBIDDEN.into_response()),
        None => None,
    }
}

/// Whether the request carries `token` as its bearer token. Digests are
/// compared rather than the tokens, so how long the comparison takes says
/// nothing about how much of the token a guess got right.
fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|given| sha256::hash(given.trim()) == sha256::hash(token))
}

async fn encode(services: &Services, out: &mut Text) {
    let server = &services.server;

    server.metrics.encode(out);

    out.family(
        "phantom_build_info",
        Kind::Gauge,
        "Always 1; the version is in the label.",
    );
    out.sample("phantom_build_info", &[("version", info::version())], 1_u64);

    let started = server
        .started
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    out.single(
        "process_start_time_seconds",
        Kind::Gauge,
        "When the server started, in seconds since the epoch.",
        started.as_secs(),
    );
    out.single(
        "phantom_uptime_seconds",
        Kind::Gauge,
        "How long the server has been running.",
        server.uptime().as_secs(),
    );

    encode_tasks(services, out);
    encode_backups(services, out).await;
    encode_database(services, out).await;
    encode_slow(&slow(services).await, out);
}

fn encode_tasks(services: &Services, out: &mut Text) {
    use phantom_service::ops::tasks::Status;

    let tasks = services.tasks.list();
    out.family(
        "phantom_tasks",
        Kind::Gauge,
        "Admin tasks of the last week, by status.",
    );
    for status in [
        Status::Scheduled,
        Status::Active,
        Status::Complete,
        Status::Failed,
        Status::Cancelled,
    ] {
        let count = tasks.iter().filter(|task| task.status == status).count();
        out.sample("phantom_tasks", &[("status", status.as_str())], count);
    }
}

async fn encode_backups(services: &Services, out: &mut Text) {
    let last = services
        .backups
        .last_backup()
        .await
        .inspect_err(|e| warn!("Reading the last database backup failed: {e}"))
        .ok()
        .flatten();

    if let Some((last_ms, size)) = last {
        out.single(
            "phantom_database_last_backup_timestamp_seconds",
            Kind::Gauge,
            "When the newest database backup was taken.",
            last_ms / 1000,
        );
        out.single(
            "phantom_database_last_backup_bytes",
            Kind::Gauge,
            "The size of the newest database backup.",
            size,
        );
    }

    if let Some(next_ms) = services.backups.next_backup_ms() {
        out.single(
            "phantom_database_next_backup_timestamp_seconds",
            Kind::Gauge,
            "When the next scheduled database backup is due.",
            next_ms / 1000,
        );
    }
}

async fn encode_database(services: &Services, out: &mut Text) {
    let db = services.db.clone();
    let stats: Result<_> = spawn_blocking(move || db.engine.stats())
        .await
        .map_err(Into::into)
        .and_then(|stats| stats);

    let stats = match stats {
        Ok(stats) => stats,
        Err(e) => {
            warn!("Reading the database's statistics failed: {e}");
            return;
        }
    };

    out.family(
        "phantom_database_cache_bytes",
        Kind::Gauge,
        "Bytes held by each of the database's block caches.",
    );
    for (cache, bytes) in &stats.cache_usage {
        out.sample("phantom_database_cache_bytes", &[("cache", cache)], *bytes);
    }

    out.family(
        "phantom_database_sst_bytes",
        Kind::Gauge,
        "Bytes of each column's SST files on disk.",
    );
    for (column, bytes) in &stats.sst_bytes {
        out.sample("phantom_database_sst_bytes", &[("column", column)], *bytes);
    }

    out.single(
        "phantom_database_memtable_bytes",
        Kind::Gauge,
        "Bytes held by the memtables.",
        stats.mem_tables,
    );
    out.single(
        "phantom_database_memtable_unflushed_bytes",
        Kind::Gauge,
        "Bytes in the memtables not yet flushed to disk.",
        stats.mem_tables_unflushed,
    );
    out.single(
        "phantom_database_table_readers_bytes",
        Kind::Gauge,
        "Bytes held by open table readers, indexes and filters included.",
        stats.table_readers,
    );
    out.single(
        "phantom_database_pending_compaction_bytes",
        Kind::Gauge,
        "Bytes compaction estimates it still has to rewrite.",
        stats.pending_compaction_bytes,
    );
    out.single(
        "phantom_database_running_compactions",
        Kind::Gauge,
        "Compactions running now.",
        stats.running_compactions,
    );
}

/// What a scrape reads with a database scan, and when it was read.
#[derive(Clone, Copy, Debug, Default)]
struct Slow {
    accounts: usize,
    active_accounts: usize,
    rooms: usize,
    queues: QueueDepths,
}

static SLOW: Mutex<Option<(Instant, Slow)>> = Mutex::new(None);

async fn slow(services: &Services) -> Slow {
    if let Some((at, slow)) = *SLOW.lock().expect("locked")
        && at.elapsed() < SLOW_TTL
    {
        return slow;
    }

    // Not held across the scans: two scrapes racing both scan, which is
    // cheaper than making one wait for the other.
    let slow = Slow {
        accounts: services.users.count().await,
        active_accounts: services.users.list_local_users().count().await,
        rooms: services.rooms.metadata.iter_ids().count().await,
        queues: services.sending.db.queue_depths().await,
    };

    SLOW.lock().expect("locked").replace((Instant::now(), slow));

    slow
}

fn encode_slow(slow: &Slow, out: &mut Text) {
    out.single(
        "phantom_local_users",
        Kind::Gauge,
        "Local accounts, deactivated ones included.",
        slow.accounts,
    );
    out.single(
        "phantom_local_users_active",
        Kind::Gauge,
        "Local accounts that can sign in.",
        slow.active_accounts,
    );
    out.single(
        "phantom_rooms",
        Kind::Gauge,
        "Rooms the server knows of.",
        slow.rooms,
    );

    out.family(
        "phantom_sending_queue_events",
        Kind::Gauge,
        "Events waiting to be sent, by destination class and state: queued, or in a transaction \
         in flight or awaiting a retry.",
    );
    for (index, class) in SendClass::ALL.into_iter().enumerate() {
        for (state, depths) in [
            ("queued", &slow.queues.queued),
            ("active", &slow.queues.active),
        ] {
            out.sample(
                "phantom_sending_queue_events",
                &[("class", class.as_str()), ("state", state)],
                depths[index],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{HeaderMap, HeaderValue, Request, StatusCode, header},
        middleware,
        routing::get,
    };
    use figment::{
        Figment,
        providers::{Format, Toml},
    };
    use phantom_core::{
        Config,
        diagnostics::log::{Log, LogLevelReloadHandles, capture},
        runtime::server::Server,
    };
    use tower::ServiceExt;

    use super::{authorized, refusal, track};

    fn server() -> Arc<Server> {
        let toml = r#"
            [global]
            server_name = "phantom.chat"
            database_path = "/var/lib/phantom"
            "#;

        let config = Config::new(&Figment::new().merge(Toml::string(toml).nested()))
            .expect("config is valid");

        let log = Log {
            reload: LogLevelReloadHandles::default(),
            capture: Arc::new(capture::State::new()),
        };

        Arc::new(Server::new(config, None, log))
    }

    #[tokio::test]
    async fn requests_are_counted_by_route_template() {
        let server = server();
        let router = Router::new()
            .route("/rooms/{room_id}", get(|| async { "ok" }))
            .fallback(|| async { StatusCode::NOT_FOUND })
            .layer(middleware::from_fn_with_state(server.clone(), track));

        for uri in ["/rooms/!a:x", "/rooms/!b:x", "/nowhere/!c:x"] {
            let request = Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("valid request");
            router.clone().oneshot(request).await.expect("answered");
        }

        let series = server.metrics.http.snapshot();
        let routes: Vec<_> = series
            .iter()
            .map(|(labels, snapshot)| {
                (
                    labels.route.as_str(),
                    labels.method,
                    labels.class,
                    snapshot.count,
                )
            })
            .collect();

        assert_eq!(
            routes,
            [
                ("/rooms/{room_id}", "GET", "2xx", 2),
                ("unmatched", "GET", "4xx", 1),
            ],
            "the raw paths never become labels"
        );
        assert_eq!(
            server
                .metrics
                .requests_handle_active
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn a_scrape_needs_the_exact_bearer_token() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, HeaderValue::from_static(value));
            headers
        };

        assert!(authorized(&with("Bearer s3cret"), "s3cret"));
        assert!(!authorized(&with("Bearer s3cre"), "s3cret"));
        assert!(!authorized(&with("Basic s3cret"), "s3cret"));
        assert!(!authorized(&HeaderMap::new(), "s3cret"));
    }

    #[test]
    fn the_main_listeners_never_serve_without_a_token() {
        let none = HeaderMap::new();
        let status = |refused: Option<axum::response::Response>| refused.map(|r| r.status());

        // A reload dropped the token: the main listeners refuse every scrape.
        assert_eq!(
            status(refusal(&none, None, true)),
            Some(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            status(refusal(&none, Some(""), true)),
            Some(StatusCode::FORBIDDEN)
        );

        // The metrics listener serves without one, as configured.
        assert_eq!(status(refusal(&none, None, false)), None);

        // With a token, both ask for it.
        for required in [true, false] {
            assert_eq!(
                status(refusal(&none, Some("s3cret"), required)),
                Some(StatusCode::UNAUTHORIZED)
            );
        }
    }
}
