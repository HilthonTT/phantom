use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::{Err, Result};
use serde::Serialize;
use serde_json::json;
use tokio::task::spawn_blocking;

use crate::router::{AdminAuth, State as RouterState};

/// # `POST /_phantom/admin/v1/config/reload`
///
/// Reads the config again from the files and overrides the server started
/// with, as SIGUSR1 does. A change the running server cannot take, such as
/// another server name, is refused and the old config kept.
pub(super) async fn reload(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    services.config.reload_running()?;

    Ok(Json(json!({})))
}

/// # `POST /_phantom/admin/v1/backup`
///
/// Backs the database up into `database_backup_path`, as a tracked task;
/// answers with the task's ID.
pub(super) async fn backup(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    if services.config.database.database_backup_path.is_none() {
        return Err!(Request(InvalidParam(
            "Backups are off; set database_backup_path to turn them on."
        )));
    }

    let db = services.db.clone();
    let task_id = services
        .tasks
        .spawn("database backup", String::new(), async move {
            spawn_blocking(move || db.engine.backup()).await??;
            Ok(json!({}))
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

#[derive(Serialize)]
pub(super) struct Task {
    id: String,
    action: &'static str,
    resource: String,
    status: &'static str,
    updated_at_ms: u64,
    result: Option<serde_json::Value>,
    error: Option<String>,
}

/// # `GET /_phantom/admin/v1/tasks`
///
/// The long operations started over the last week, newest first.
pub(super) async fn tasks(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let mut tasks: Vec<Task> = services
        .tasks
        .list()
        .into_iter()
        .map(|task| Task {
            id: task.id.to_string(),
            action: task.action,
            resource: task.resource_id,
            status: task.status.as_str(),
            updated_at_ms: task.timestamp_ms,
            result: task.result,
            error: task.error,
        })
        .collect();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.updated_at_ms));

    Ok(Json(tasks))
}
