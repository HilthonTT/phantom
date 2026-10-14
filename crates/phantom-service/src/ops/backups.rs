//! Database backups: starting one as a tracked task, and taking them on the
//! schedule `database_backup_interval_s` sets.
//!
//! The schedule counts from the newest backup in `database_backup_path`
//! rather than from startup, so a server restarted more often than the
//! interval still backs up. Retention is the backup engine's: each backup
//! prunes all but the newest `database_backups_to_keep`.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use phantom_core::{
    Err, Result, debug, implement, info, runtime::server::Server, time::now_millis, warn,
};
use phantom_database::Database;
use serde_json::json;
use tokio::{task::spawn_blocking, time::sleep};

use crate::{
    Dep,
    ops::tasks::{self, TaskId},
};

/// The action a backup task is listed under.
pub const ACTION: &str = "database backup";

/// How often the worker looks at the config again while backups are off or
/// the next one is far off, so a reload that changes the schedule is noticed.
const RECHECK: Duration = Duration::from_mins(5);

pub struct Service {
    services: Services,

    /// When the scheduler last started a backup, or tried to.
    last_attempt_ms: AtomicU64,

    /// When the next scheduled backup is due; 0 when none is scheduled.
    next_ms: AtomicU64,
}

struct Services {
    server: Arc<Server>,
    db: Arc<Database>,
    tasks: Dep<tasks::Service>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                db: args.db.clone(),
                tasks: args.depend::<tasks::Service>(),
            },
            last_attempt_ms: AtomicU64::new(0),
            next_ms: AtomicU64::new(0),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        loop {
            let wait = self.tick().await;

            tokio::select! {
                () = sleep(wait) => {},
                () = self.services.server.until_shutdown() => return Ok(()),
            }
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Starts a backup as a tracked task, answering with its ID. A backup runs on
/// a blocking thread RocksDB gives no way to interrupt, so the task cannot be
/// cancelled.
#[implement(Service)]
pub fn start(&self) -> Result<TaskId> {
    if !self.enabled() {
        return Err!(Request(InvalidParam(
            "Backups are off; set database_backup_path to turn them on."
        )));
    }

    // The engine refuses a second backup at once as well; this answers with
    // why instead of starting a task bound to fail.
    if self.services.tasks.has_nonterminal(ACTION, "") {
        return Err!(Conflict("A database backup is already running."));
    }

    let db = self.services.db.clone();
    let task_id = self
        .services
        .tasks
        .spawn_uncancellable(ACTION, String::new(), async move {
            spawn_blocking(move || db.engine.backup()).await??;
            Ok(json!({}))
        });

    Ok(task_id)
}

/// When the next scheduled backup is due, in milliseconds since the epoch;
/// none when backups are not scheduled.
#[implement(Service)]
#[must_use]
pub fn next_backup_ms(&self) -> Option<u64> {
    Some(self.next_ms.load(Ordering::Relaxed)).filter(|&ms| ms > 0)
}

/// The newest backup's time in milliseconds since the epoch, and its size;
/// none when backups are off or none was made yet.
#[implement(Service)]
pub async fn last_backup(&self) -> Result<Option<(u64, u64)>> {
    let db = self.services.db.clone();
    let last = spawn_blocking(move || db.engine.last_backup()).await??;

    Ok(last.map(|(secs, size)| {
        let ms = u64::try_from(secs).unwrap_or_default().saturating_mul(1000);

        (ms, size)
    }))
}

#[implement(Service)]
fn enabled(&self) -> bool {
    self.services
        .server
        .config
        .database
        .database_backup_path
        .as_ref()
        .is_some_and(|path| !path.as_os_str().is_empty())
}

/// One pass of the scheduler: starts a backup when one is due, and answers
/// how long to wait before the next pass.
#[implement(Service)]
async fn tick(&self) -> Duration {
    let interval_s = self
        .services
        .server
        .config
        .database
        .database_backup_interval_s;

    if interval_s == 0 || !self.enabled() {
        self.next_ms.store(0, Ordering::Relaxed);
        return RECHECK;
    }

    // Unreadable, the backup directory reads as holding no backup, which
    // makes one due; that backup then reports what is wrong with it.
    let last_backup_ms = self
        .last_backup()
        .await
        .inspect_err(|e| warn!("Reading the last database backup failed: {e}"))
        .ok()
        .flatten()
        .map(|(ms, _)| ms);

    let last_attempt_ms = Some(self.last_attempt_ms.load(Ordering::Relaxed)).filter(|&ms| ms > 0);
    let interval_ms = interval_s.saturating_mul(1000);
    let now = now_millis();

    let due = next_due_ms(last_backup_ms, last_attempt_ms, interval_ms, now);
    if due > now {
        self.next_ms.store(due, Ordering::Relaxed);
        return wait_until(due, now);
    }

    self.last_attempt_ms.store(now, Ordering::Relaxed);
    let next = now.saturating_add(interval_ms);
    self.next_ms.store(next, Ordering::Relaxed);

    match self.start() {
        Ok(task_id) => info!(%task_id, "Starting a scheduled database backup"),
        Err(e) => debug!("Skipping a scheduled database backup: {e}"),
    }

    wait_until(next, now)
}

/// When the next backup is due: an interval after the newest backup or the
/// newest attempt at one, whichever is later, so a failing backup is retried
/// an interval later rather than on every pass. With neither, one is due now.
fn next_due_ms(
    last_backup_ms: Option<u64>,
    last_attempt_ms: Option<u64>,
    interval_ms: u64,
    now_ms: u64,
) -> u64 {
    last_backup_ms
        .max(last_attempt_ms)
        .map_or(now_ms, |anchor| anchor.saturating_add(interval_ms))
}

/// How long to sleep before looking again: until `due`, but no longer than
/// [`RECHECK`].
fn wait_until(due_ms: u64, now_ms: u64) -> Duration {
    Duration::from_millis(due_ms.saturating_sub(now_ms)).min(RECHECK)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{RECHECK, next_due_ms, wait_until};

    const HOUR: u64 = 60 * 60 * 1000;

    #[test]
    fn with_no_backup_one_is_due_now() {
        assert_eq!(next_due_ms(None, None, HOUR, 5_000), 5_000);
    }

    #[test]
    fn the_next_backup_is_an_interval_after_the_last() {
        assert_eq!(next_due_ms(Some(1_000), None, HOUR, 2_000), 1_000 + HOUR);
    }

    #[test]
    fn an_overdue_backup_is_due_now() {
        let due = next_due_ms(Some(1_000), None, HOUR, 10 * HOUR);
        assert!(due <= 10 * HOUR, "overdue reads as due");
    }

    #[test]
    fn a_failed_attempt_waits_an_interval_before_trying_again() {
        // The newest backup is old, but an attempt was just made and failed.
        let due = next_due_ms(Some(1_000), Some(5 * HOUR), HOUR, 5 * HOUR + 10);
        assert_eq!(due, 6 * HOUR);
    }

    #[test]
    fn a_backup_newer_than_the_attempt_counts_from_the_backup() {
        // An admin backed up by hand after the scheduler's last attempt.
        let due = next_due_ms(Some(3 * HOUR), Some(2 * HOUR), HOUR, 3 * HOUR);
        assert_eq!(due, 4 * HOUR);
    }

    #[test]
    fn waits_are_capped_so_a_reload_is_noticed() {
        assert_eq!(wait_until(10 * HOUR, 0), RECHECK);
        assert_eq!(wait_until(1_500, 1_000), Duration::from_millis(500));
        assert_eq!(wait_until(1_000, 2_000), Duration::ZERO);
    }
}
