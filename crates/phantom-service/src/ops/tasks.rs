//! The admin's long-running operations: room shutdowns and deletions,
//! backups, purges. Each runs as its own tokio task, tracked here so the admin
//! API can list it, and an admin can ask for it to stop.
//!
//! How a task stops depends on how it was spawned, because not every task
//! can be dropped half way safely:
//!
//! - [`spawn`](Service::spawn): dropped at its next await point, the same
//!   thing shutdown already does to every task.
//! - [`spawn_cancellable`](Service::spawn_cancellable): handed a
//!   [`CancelToken`] it checks between steps it can stop between, so it ends
//!   at a point it chose.
//! - [`spawn_uncancellable`](Service::spawn_uncancellable): refuses, for work
//!   that would carry on regardless, such as a backup running on a blocking
//!   thread that nothing can interrupt.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use arrayvec::ArrayString;
use async_trait::async_trait;
use phantom_core::{
    Err, Result, err, implement, rand::string_array, runtime::server::Server, time::now_millis,
};
use serde_json::Value as JsonValue;
use tokio::{sync::watch, task::JoinHandle, time::sleep};

const TASK_ID_LEN: usize = 16;

const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

const CAPACITY: usize = 1024;

const GC_INTERVAL: Duration = Duration::from_hours(1);

pub type TaskId = ArrayString<TASK_ID_LEN>;

pub struct Service {
    services: Services,
    tasks: StdMutex<BTreeMap<TaskId, Task>>,
}

struct Services {
    server: Arc<Server>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Scheduled,
    Active,
    Complete,
    Failed,
    Cancelled,
}

/// How a task answers a request to cancel it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cancel {
    /// Dropped at its next await point.
    Abort,

    /// Told through its [`CancelToken`]; it stops at the next point it checks.
    Cooperative,

    /// Cannot be stopped once started.
    Never,
}

/// Tells a cooperative task it has been asked to stop. The task checks it at
/// the points it can stop at without leaving a half-done step behind.
#[derive(Clone, Debug)]
pub struct CancelToken(Arc<watch::Sender<bool>>);

/// A tracked task's public snapshot, cloned out from under the lock.
#[derive(Clone, Debug)]
pub struct TaskInfo {
    pub id: TaskId,
    pub action: &'static str,
    pub resource_id: String,
    pub status: Status,
    pub timestamp_ms: u64,
    pub result: Option<JsonValue>,
    pub error: Option<String>,
    pub cancel: Cancel,

    /// An admin asked for the task to stop, and it has not finished since.
    pub cancel_requested: bool,
}

struct Task {
    action: &'static str,
    resource_id: String,
    status: Status,

    /// When the status last changed, so a finished task is kept for the
    /// retention window from when it finished, not from when it started.
    timestamp_ms: u64,
    result: Option<JsonValue>,
    error: Option<String>,
    handle: Option<JoinHandle<()>>,
    cancel: Cancel,
    token: CancelToken,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
            },
            tasks: StdMutex::new(BTreeMap::new()),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        loop {
            self.prune();

            tokio::select! {
                () = sleep(GC_INTERVAL) => {},
                () = self.services.server.until_shutdown() => return Ok(()),
            }
        }
    }

    fn interrupt(&self) {
        self.abort_all();
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Spawn `work` on the runtime as a tracked task, returning its id. The record
/// transitions Scheduled -> Active -> Complete/Failed; `work`'s `Ok` value is
/// stored as the result, its `Err` as the error string.
///
/// Cancelling it drops `work` at its next await point, so whatever it had
/// done by then stays done; spawn work that cannot stop just anywhere with
/// [`spawn_cancellable`](Self::spawn_cancellable) instead.
#[implement(Service)]
pub fn spawn<F>(self: &Arc<Self>, action: &'static str, resource_id: String, work: F) -> TaskId
where
    F: Future<Output = Result<JsonValue>> + Send + 'static,
{
    self.spawn_with(action, resource_id, Cancel::Abort, |_| work)
}

/// Spawn a tracked task that stops itself when cancelled: `work` is given a
/// [`CancelToken`] and is expected to check it between steps. A task that
/// returns an error after it was cancelled is recorded as cancelled; one that
/// finishes regardless is recorded as complete.
#[implement(Service)]
pub fn spawn_cancellable<F, W>(
    self: &Arc<Self>,
    action: &'static str,
    resource_id: String,
    work: W,
) -> TaskId
where
    W: FnOnce(CancelToken) -> F,
    F: Future<Output = Result<JsonValue>> + Send + 'static,
{
    self.spawn_with(action, resource_id, Cancel::Cooperative, work)
}

/// Spawn a tracked task that refuses to be cancelled.
#[implement(Service)]
pub fn spawn_uncancellable<F>(
    self: &Arc<Self>,
    action: &'static str,
    resource_id: String,
    work: F,
) -> TaskId
where
    F: Future<Output = Result<JsonValue>> + Send + 'static,
{
    self.spawn_with(action, resource_id, Cancel::Never, |_| work)
}

#[implement(Service)]
fn spawn_with<F, W>(
    self: &Arc<Self>,
    action: &'static str,
    resource_id: String,
    cancel: Cancel,
    work: W,
) -> TaskId
where
    W: FnOnce(CancelToken) -> F,
    F: Future<Output = Result<JsonValue>> + Send + 'static,
{
    let id = string_array::<TASK_ID_LEN>();
    let token = CancelToken::new();
    let work = work(token.clone());

    // Hold the lock across the spawn+insert so the task cannot mark itself
    // Active before its record exists.
    let mut tasks = self.tasks.lock().expect("locked");
    let this = Arc::clone(self);
    let task_id = id;
    let task_token = token.clone();
    let handle = self.services.server.runtime().spawn(async move {
        this.set_active(&task_id);
        let outcome = match cancel {
            Cancel::Abort => tokio::select! {
                biased;
                () = task_token.cancelled() => Err(cancelled()),
                outcome = work => outcome,
            },
            Cancel::Cooperative | Cancel::Never => work.await,
        };
        this.finish(&task_id, outcome, task_token.is_cancelled());
    });

    tasks.insert(
        id,
        Task {
            action,
            resource_id,
            status: Status::Scheduled,
            timestamp_ms: now_millis(),
            result: None,
            error: None,
            handle: Some(handle),
            cancel,
            token,
        },
    );

    id
}

/// Ask the task with this id to stop, answering with its record as it is
/// now. A cooperative task may take until its next check to stop, so the
/// record can still say active, with `cancel_requested` set.
#[implement(Service)]
pub fn cancel(&self, id: &str) -> Result<TaskInfo> {
    let tasks = self.tasks.lock().expect("locked");
    let Some((id, task)) = tasks.get_key_value(id) else {
        return Err!(Request(NotFound("No task has the ID {id}.")));
    };

    if task.status.is_terminal() {
        return Err!(Conflict("The task has already finished."));
    }

    if task.cancel == Cancel::Never {
        return Err!(Conflict(
            "A {} cannot be stopped once started.",
            task.action
        ));
    }

    task.token.cancel();

    Ok(task.info(id))
}

/// The task with this id, if it is still tracked.
#[implement(Service)]
pub fn get(&self, id: &str) -> Option<TaskInfo> {
    self.tasks
        .lock()
        .expect("locked")
        .get_key_value(id)
        .map(|(id, task)| task.info(id))
}

/// Every tracked task acting on `resource_id`, newest ordering not guaranteed.
#[implement(Service)]
pub fn by_resource(&self, resource_id: &str) -> Vec<TaskInfo> {
    self.tasks
        .lock()
        .expect("locked")
        .iter()
        .filter(|(_, task)| task.resource_id.as_str() == resource_id)
        .map(|(id, task)| task.info(id))
        .collect()
}

/// Whether a nonterminal task matches both `action` and `resource_id`.
#[implement(Service)]
pub fn has_nonterminal(&self, action: &str, resource_id: &str) -> bool {
    self.tasks
        .lock()
        .expect("locked")
        .values()
        .any(|task| matches_nonterminal(task, action, resource_id))
}

fn matches_nonterminal(task: &Task, action: &str, resource_id: &str) -> bool {
    task.action == action && task.resource_id == resource_id && !task.status.is_terminal()
}

/// Every tracked task; callers filter by action or status.
#[implement(Service)]
pub fn list(&self) -> Vec<TaskInfo> {
    self.tasks
        .lock()
        .expect("locked")
        .iter()
        .map(|(id, task)| task.info(id))
        .collect()
}

#[implement(Service)]
fn set_active(&self, id: &str) {
    if let Some(task) = self.tasks.lock().expect("locked").get_mut(id) {
        task.status = Status::Active;
        task.timestamp_ms = now_millis();
    }
}

#[implement(Service)]
fn finish(&self, id: &str, outcome: Result<JsonValue>, cancel_requested: bool) {
    let mut tasks = self.tasks.lock().expect("locked");
    let Some(task) = tasks.get_mut(id) else {
        return;
    };

    task.status = outcome_status(outcome.is_ok(), cancel_requested);
    match outcome {
        Ok(value) => task.result = Some(value),
        Err(error) => task.error = Some(error.to_string()),
    }
    task.timestamp_ms = now_millis();
}

/// What a task that ended this way is recorded as. One that finished despite
/// being asked to stop did all its work, so it is complete; one that failed
/// after being asked is taken to have stopped because it was asked.
fn outcome_status(ok: bool, cancel_requested: bool) -> Status {
    match (ok, cancel_requested) {
        (true, _) => Status::Complete,
        (false, true) => Status::Cancelled,
        (false, false) => Status::Failed,
    }
}

fn cancelled() -> phantom_core::Error {
    err!("Cancelled by an admin.")
}

#[implement(Service)]
fn prune(&self) {
    let now = now_millis();

    prune_tasks(&mut self.tasks.lock().expect("locked"), now);
}

#[implement(Service)]
fn abort_all(&self) {
    self.tasks
        .lock()
        .expect("locked")
        .values()
        .filter_map(|task| task.handle.as_ref())
        .for_each(JoinHandle::abort);
}

impl Status {
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Failed | Self::Cancelled)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Active => "active",
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl Task {
    fn info(&self, id: &TaskId) -> TaskInfo {
        TaskInfo {
            id: *id,
            action: self.action,
            resource_id: self.resource_id.clone(),
            status: self.status,
            timestamp_ms: self.timestamp_ms,
            result: self.result.clone(),
            error: self.error.clone(),
            cancel: self.cancel,
            cancel_requested: self.token.is_cancelled() && !self.status.is_terminal(),
        }
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    /// A token nothing cancels, for running cancellable work outside a task.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(watch::Sender::new(false)))
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Fails once the task has been asked to stop; `?` it at each point the
    /// task can stop at.
    pub fn check(&self) -> Result {
        if self.is_cancelled() {
            return Err(cancelled());
        }

        Ok(())
    }

    /// Resolves once the task has been asked to stop, for racing a wait that
    /// could otherwise outlast the request.
    pub async fn cancelled(&self) {
        // The sender lives in the token, so the channel cannot close under
        // the wait; an error here would only mean it had.
        self.0
            .subscribe()
            .wait_for(|&cancelled| cancelled)
            .await
            .ok();
    }

    fn cancel(&self) {
        self.0.send_replace(true);
    }
}

/// Drop terminal tasks past the retention window, then cap the survivors.
fn prune_tasks(tasks: &mut BTreeMap<TaskId, Task>, now_ms: u64) {
    tasks.retain(|_, task| {
        !task.status.is_terminal() || now_ms.saturating_sub(task.timestamp_ms) < RETENTION_MS
    });

    let mut timestamps: Vec<u64> = tasks
        .values()
        .filter(|task| task.status.is_terminal())
        .map(|task| task.timestamp_ms)
        .collect();

    if timestamps.len() <= CAPACITY {
        return;
    }

    timestamps.sort_unstable();

    let cutoff = timestamps[timestamps.len().saturating_sub(CAPACITY)];

    tasks.retain(|_, task| !task.status.is_terminal() || task.timestamp_ms >= cutoff);
}

#[cfg(test)]
mod tests {
    use super::{CancelToken, Status, outcome_status};

    #[test]
    fn an_outcome_after_a_cancel_request_reads_as_cancelled_only_if_it_failed() {
        assert_eq!(outcome_status(true, false), Status::Complete);
        assert_eq!(outcome_status(false, false), Status::Failed);
        assert_eq!(outcome_status(false, true), Status::Cancelled);
        assert_eq!(outcome_status(true, true), Status::Complete);
        assert!(Status::Cancelled.is_terminal());
    }

    #[tokio::test]
    async fn a_token_wakes_its_waiters_and_fails_its_checks() {
        let token = CancelToken::new();
        assert!(token.check().is_ok());

        let waiter = tokio::spawn({
            let token = token.clone();
            async move { token.cancelled().await }
        });

        token.cancel();
        waiter.await.expect("woken");
        assert!(token.check().is_err());

        // A wait that starts after the cancel resolves at once.
        token.cancelled().await;
    }
}
