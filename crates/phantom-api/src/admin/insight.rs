//! The admin API's views of the server's internals: its services, the
//! servers it federates with, its media store, its log and its abuse reports.

use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use futures::StreamExt;
use phantom_core::{Err, Result, stream::ReadyExt};
use phantom_service::net::sending::Destination;
use ruma::{OwnedMxcUri, OwnedServerName};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct ServiceRow {
    name: String,

    /// "running", "finished" (returned, as a service without background
    /// work does at once), "failed", or "not started".
    status: &'static str,
    started_at_ms: Option<u64>,
    stopped_at_ms: Option<u64>,
    restarts: u32,
    error: Option<String>,
}

/// # `GET /_phantom/admin/v1/services`
///
/// Every registered service, with what became of its worker.
pub(super) async fn services(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let rows: Vec<ServiceRow> = services
        .workers()
        .into_iter()
        .map(|(name, state)| match state {
            Some(state) => ServiceRow {
                name,
                status: state.status.as_str(),
                started_at_ms: Some(state.started_ms),
                stopped_at_ms: state.stopped_ms,
                restarts: state.restarts,
                error: state.error,
            },
            None => ServiceRow {
                name,
                status: "not started",
                started_at_ms: None,
                stopped_at_ms: None,
                restarts: 0,
                error: None,
            },
        })
        .collect();

    Ok(Json(rows))
}

#[derive(Serialize)]
pub(super) struct Peer {
    server: OwnedServerName,

    /// Rooms this server shares with it.
    rooms: usize,

    /// When it last answered a request since startup, in milliseconds.
    last_contact_ms: Option<u64>,

    /// Set while requests to it are held back after failures.
    backoff: Option<Backoff>,

    /// Where its name resolved to, while the resolution is cached.
    resolved: Option<String>,

    /// Transactions to it now in flight.
    sending: usize,

    /// Events waiting for the transaction in flight to finish.
    queued: usize,

    /// Whether the config file or the runtime block list refuses it.
    blocked: bool,

    /// When a request to it last failed since startup, and why.
    last_failure_ms: Option<u64>,
    last_error: Option<String>,

    /// Requests that failed since the last one it answered.
    consecutive_failures: u32,

    /// When it last sent this server a transaction, since startup.
    last_inbound_ms: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct Backoff {
    permanent: bool,
    since_ms: u64,
    delay_secs: u64,
}

/// # `GET /_phantom/admin/v1/federation`
///
/// Every server this one shares a room with, and how talking to it goes.
pub(super) async fn federation(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let contacts = services.federation.last_contacts();
    let backoffs = services.federation.peer_backoffs().await;
    let health = services.federation.peer_health();

    let mut resolved: BTreeMap<String, String> = BTreeMap::new();
    services
        .resolver
        .cache
        .destinations()
        .ready_for_each(|(name, cached)| {
            resolved.insert(name.to_owned(), cached.host);
        })
        .await;

    let mut sending: BTreeMap<OwnedServerName, usize> = BTreeMap::new();
    services
        .sending
        .db
        .active_requests()
        .ready_for_each(|(_, _, dest)| {
            if let Destination::Federation(server) = dest {
                *sending.entry(server).or_default() += 1;
            }
        })
        .await;

    let known: Vec<(OwnedServerName, usize)> = services
        .rooms
        .state_cache
        .known_servers()
        .await
        .into_iter()
        .filter(|(server, _)| !services.server_state.server_is_ours(server))
        .collect();

    let mut queued: BTreeMap<OwnedServerName, usize> = BTreeMap::new();
    for (server, _) in &known {
        let dest = Destination::Federation(server.clone());
        let count = services.sending.db.queued_requests(&dest).count().await;
        queued.insert(server.clone(), count);
    }

    let peers: Vec<Peer> = known
        .into_iter()
        .map(|(server, rooms)| {
            let health = health.get(&server).cloned().unwrap_or_default();

            Peer {
                rooms,
                last_contact_ms: contacts.get(&server).map(|secs| secs.saturating_mul(1000)),
                backoff: backoffs.get(&server).map(|backoff| Backoff {
                    permanent: backoff.class
                        == phantom_service::net::federation::Classification::Permanent,
                    since_ms: backoff.anchor_secs.saturating_mul(1000),
                    delay_secs: backoff.delay_secs,
                }),
                resolved: resolved.get(server.as_str()).cloned(),
                sending: sending.get(&server).copied().unwrap_or(0),
                queued: queued.get(&server).copied().unwrap_or(0),
                blocked: services.moderation.forbids(
                    &server,
                    phantom_service::ops::moderation::Restriction::Federation,
                ),
                last_failure_ms: health.last_failure_ms,
                last_error: health.last_error,
                consecutive_failures: health.consecutive_failures,
                last_inbound_ms: health.last_inbound_ms,
                server,
            }
        })
        .collect();

    Ok(Json(peers))
}

#[derive(Serialize)]
pub(super) struct MediaRow {
    mxc: OwnedMxcUri,
    size: u64,
    content_type: Option<String>,
    created_at_ms: u64,
    thumbnails: usize,
    uploader: Option<String>,
    local: bool,

    /// Whether the file is quarantined: kept, but answered as missing.
    quarantined: bool,
}

/// # `GET /_phantom/admin/v1/media`
///
/// Every file the media store holds, local uploads and remote copies.
pub(super) async fn media(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let quarantined: BTreeSet<OwnedMxcUri> = services
        .media
        .quarantined()
        .await
        .into_iter()
        .map(|record| record.mxc)
        .collect();

    let rows: Vec<MediaRow> = services
        .media
        .list()
        .await
        .into_iter()
        .map(|media| MediaRow {
            quarantined: quarantined.contains(&media.mxc),
            local: media
                .mxc
                .server_name()
                .is_ok_and(|server| services.server_state.server_is_ours(server)),
            size: media.meta.size,
            content_type: media.meta.content_type,
            created_at_ms: media.meta.created.saturating_mul(1000),
            thumbnails: media.thumbnails,
            uploader: media.uploader.map(|user| user.to_string()),
            mxc: media.mxc,
        })
        .collect();

    Ok(Json(rows))
}

/// # `DELETE /_phantom/admin/v1/media/{server_name}/{media_id}`
///
/// Deletes a file and its thumbnails from the store.
pub(super) async fn delete_media(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path((server, media_id)): Path<(String, String)>,
) -> Result<impl IntoResponse> {
    let mxc = OwnedMxcUri::from(format!("mxc://{server}/{media_id}"));
    services.media.delete(&mxc).await?;

    Ok(Json(json!({})))
}

/// # `DELETE /_phantom/admin/v1/federation/{server_name}/media`
///
/// Deletes every copy of another server's media this one holds, answering
/// with how many files went.
pub(super) async fn purge_remote_media(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(server): Path<OwnedServerName>,
) -> Result<impl IntoResponse> {
    let removed = services.media.delete_from_server(&server).await?;

    Ok(Json(json!({ "removed": removed })))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct LogQuery {
    limit: Option<usize>,

    /// Only the lines after the one with this `seq`, for following the log.
    since: Option<u64>,
}

#[derive(Serialize)]
pub(super) struct LogLine {
    seq: u64,
    at_ms: u64,
    level: String,
    target: String,
    span: String,
    message: String,
}

/// # `GET /_phantom/admin/v1/logs?limit=&since=`
///
/// The server's recent log at info level and above, newest first; the
/// server keeps the last thousand lines. Each line carries a `seq`, and
/// `since` answers with only the lines after it, which is how the console
/// follows the log.
pub(super) async fn logs(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Query(query): Query<LogQuery>,
) -> Result<impl IntoResponse> {
    let limit = query.limit.unwrap_or(usize::MAX);
    let lines = match query.since {
        Some(since) => services.logs.since(since, limit),
        None => services.logs.recent(limit),
    };

    let lines: Vec<LogLine> = lines
        .into_iter()
        .map(|line| LogLine {
            seq: line.seq,
            at_ms: line.at_ms,
            level: line.level.to_string(),
            target: line.target,
            span: line.span,
            message: line.message,
        })
        .collect();

    Ok(Json(lines))
}

#[derive(Deserialize, Serialize)]
pub(super) struct LogCapture {
    /// "info", or "debug" while phantom's own debug lines are kept too.
    level: String,
}

/// # `GET /_phantom/admin/v1/logs/capture`
///
/// The lowest level the log keeps.
pub(super) async fn log_capture(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    Ok(Json(capture_level(services.logs.captures_debug())))
}

/// # `PUT /_phantom/admin/v1/logs/capture`
///
/// Sets the lowest level the log keeps: "debug" adds phantom's own debug
/// lines until it is set back to "info" or the server restarts.
pub(super) async fn set_log_capture(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: LogCapture = super::body(&body)?;
    let debug = match body.level.as_str() {
        "debug" => true,
        "info" => false,
        level => {
            return Err!(Request(InvalidParam(
                "The log keeps \"info\" or \"debug\", not {level:?}."
            )));
        }
    };

    services.logs.capture_debug(debug);

    Ok(Json(capture_level(debug)))
}

fn capture_level(debug: bool) -> LogCapture {
    LogCapture {
        level: if debug { "debug" } else { "info" }.to_owned(),
    }
}

#[derive(Serialize)]
pub(super) struct ReportRow {
    id: String,
    at_ms: u64,
    kind: &'static str,
    reporter: String,
    room_id: Option<String>,
    event_id: Option<String>,
    user_id: Option<String>,
    reason: String,
}

/// # `GET /_phantom/admin/v1/reports`
///
/// Every open abuse report, newest first.
pub(super) async fn reports(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let rows: Vec<ReportRow> = services
        .reports
        .list()
        .await
        .into_iter()
        .map(|report| ReportRow {
            kind: report.kind(),
            id: report.id,
            at_ms: report.at_ms,
            reporter: report.reporter.to_string(),
            room_id: report.room_id.map(|id| id.to_string()),
            event_id: report.event_id.map(|id| id.to_string()),
            user_id: report.user_id.map(|id| id.to_string()),
            reason: report.reason,
        })
        .collect();

    Ok(Json(rows))
}

/// # `DELETE /_phantom/admin/v1/reports/{id}`
///
/// Closes a report once it has been dealt with.
pub(super) async fn dismiss_report(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    services.reports.dismiss(&id).await?;

    Ok(Json(json!({})))
}
