use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use phantom_core::Result;
use phantom_service::ops::tasks::{Cancel, TaskInfo};
use serde::Serialize;
use serde_json::json;

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
    let task_id = services.backups.start()?;

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

    /// Whether the task can be asked to stop, and whether it has been.
    cancellable: bool,
    cancel_requested: bool,
}

impl From<TaskInfo> for Task {
    fn from(task: TaskInfo) -> Self {
        Self {
            id: task.id.to_string(),
            action: task.action,
            resource: task.resource_id,
            status: task.status.as_str(),
            updated_at_ms: task.timestamp_ms,
            result: task.result,
            error: task.error,
            cancellable: task.cancel != Cancel::Never && !task.status.is_terminal(),
            cancel_requested: task.cancel_requested,
        }
    }
}

/// # `GET /_phantom/admin/v1/tasks`
///
/// The long operations started over the last week, newest first.
pub(super) async fn tasks(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let mut tasks: Vec<Task> = services.tasks.list().into_iter().map(Task::from).collect();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.updated_at_ms));

    Ok(Json(tasks))
}

/// # `POST /_phantom/admin/v1/tasks/{id}/cancel`
///
/// Asks a running task to stop, answering with the task as it is now. A room
/// shutdown or deletion stops at its next safe point, so it can still read as
/// active, with `cancel_requested` set; a backup cannot be stopped.
pub(super) async fn cancel_task(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(id): Path<String>,
) -> Result<impl IntoResponse> {
    let task = services.tasks.cancel(&id)?;

    Ok(Json(Task::from(task)))
}
