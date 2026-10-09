use std::{path::Path, time::UNIX_EPOCH};

use axum::{Json, extract::State, response::IntoResponse};
use futures::StreamExt;
use phantom_core::{Result, diagnostics::info, stream::ReadyExt};
use serde::Serialize;

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct Stats {
    server_name: String,
    version: String,

    started_at_ms: u64,
    uptime_secs: u64,

    /// Every local account, deactivated ones included, and those that can
    /// still sign in.
    local_users: usize,
    active_local_users: usize,

    rooms: usize,
    appservices: usize,

    /// The size on disk of the database directory's files: tables, the
    /// write-ahead log and RocksDB's own logs, but not the media store kept in
    /// a directory beneath it.
    database_bytes: u64,

    federation: bool,
    registration: bool,
    registration_token: bool,
    read_only: bool,
}

/// # `GET /_phantom/admin/v1/stats`
///
/// The server at a glance, for the console's overview.
pub(super) async fn stats(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let server = &services.server;
    let config = &services.config;

    let local_users = services
        .users
        .stream()
        .ready_filter(|user_id| services.server_state.user_is_local(user_id))
        .count()
        .await;

    let database_bytes = files_size(&config.database.database_path).await;

    let started_at_ms = server
        .started
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default();

    Ok(Json(Stats {
        server_name: server.name.to_string(),
        version: format!("{} {}", info::name(), info::version()),
        started_at_ms,
        uptime_secs: server.uptime().as_secs(),
        local_users,
        active_local_users: services.users.list_local_users().count().await,
        rooms: services.rooms.metadata.iter_ids().count().await,
        appservices: services.appservice.iter_ids().await.len(),
        database_bytes,
        federation: config.federation.allow_federation,
        registration: config.auth.allow_registration,
        registration_token: services.registration_tokens.is_enabled().await,
        read_only: config.database.rocksdb_read_only,
    }))
}

/// Sums the sizes of the files directly in dir; an unreadable entry counts as
/// empty, since a size is all this is for.
async fn files_size(dir: &Path) -> u64 {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return 0;
    };

    let mut total = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(meta) = entry.metadata().await
            && meta.is_file()
        {
            total += meta.len();
        }
    }

    total
}
