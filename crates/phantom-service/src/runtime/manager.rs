use std::{
    collections::BTreeMap,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use futures::{FutureExt, TryFutureExt};
use phantom_core::{
    Err, Error, Result, debug, debug_warn, error, runtime::server::Server, time, time::now_millis,
    trace, warn,
};
use tokio::{
    sync::{Mutex, MutexGuard},
    task::{JoinHandle, JoinSet},
    time::sleep,
};

use super::{contract::Service, registry::Map, services::Services};

pub struct Manager {
    manager: Mutex<Option<JoinHandle<Result<()>>>>,
    workers: Mutex<Workers>,
    server: Arc<Server>,
    service: Arc<Map>,

    /// What became of each service's worker, by service name, for the admin
    /// API; the manager otherwise only logs it. Shared with `Services`, which
    /// reads it without the manager's lock, held by `poll` for the server's
    /// whole run.
    states: WorkerStates,
}

pub(crate) type WorkerStates = Arc<StdMutex<BTreeMap<String, WorkerState>>>;

/// A service worker's lifecycle as the manager has seen it.
#[derive(Clone, Debug)]
pub struct WorkerState {
    pub status: WorkerStatus,

    /// When the worker last started, and when it last stopped, in
    /// milliseconds since the epoch.
    pub started_ms: u64,
    pub stopped_ms: Option<u64>,

    /// How often it was restarted after a panic.
    pub restarts: u32,

    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerStatus {
    Running,

    /// Returned without error; a service with no background work returns at
    /// once.
    Finished,

    /// Returned an error, or panicked and is waiting to restart.
    Failed,
}

impl WorkerStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Failed => "failed",
        }
    }
}

type Workers = JoinSet<WorkerResult>;
type WorkerResult = (Arc<dyn Service>, Result<()>);
type WorkersLocked<'a> = MutexGuard<'a, Workers>;

const RESTART_DELAY_MS: u64 = 2500;

impl Manager {
    pub(super) fn new(services: &Services) -> Arc<Self> {
        Arc::new(Self {
            manager: Mutex::new(None),
            workers: Mutex::new(JoinSet::new()),
            server: services.server.clone(),
            service: services.service.clone(),
            states: services.worker_states.clone(),
        })
    }

    fn set_state(&self, name: &str, update: impl FnOnce(Option<&WorkerState>) -> WorkerState) {
        let mut states = self.states.lock().expect("locked");
        let next = update(states.get(name));
        states.insert(name.to_owned(), next);
    }

    pub(super) async fn poll(&self) -> Result<()> {
        if let Some(manager) = &mut *self.manager.lock().await {
            trace!("Polling service manager...");
            return manager.await?;
        }

        Ok(())
    }

    pub(super) async fn start(self: Arc<Self>) -> Result<()> {
        let mut workers = self.workers.lock().await;

        debug!("Starting service manager...");
        let self_ = self.clone();
        _ = self.manager.lock().await.insert(
            self.server
                .runtime()
                .spawn(async move { self_.worker().await }),
        );

        let services: Vec<Arc<dyn Service>> = self
            .service
            .read()
            .expect("locked for reading")
            .values()
            .map(|val| val.0.upgrade())
            .map(|arc| arc.expect("services available for manager startup"))
            .collect();

        debug!("Starting service workers...");
        for service in services {
            self.start_worker(&mut workers, &service).await?;
        }

        Ok(())
    }

    pub(super) async fn stop(&self) {
        if let Some(manager) = self.manager.lock().await.take() {
            debug!("Waiting for service manager...");
            if let Err(e) = manager.await {
                error!("Manager shutdown error: {e:?}");
            }
        }
    }

    async fn worker(&self) -> Result<()> {
        loop {
            let mut workers = self.workers.lock().await;
            tokio::select! {
                result = workers.join_next() => match result {
                    Some(Ok(result)) => self.handle_result(&mut workers, result).await?,
                    Some(Err(error)) => self.handle_abort(&mut workers, Error::from(error)).await?,
                    None => break,
                }
            }
        }

        debug!("Worker manager finished");
        Ok(())
    }

    async fn handle_abort(&self, _workers: &mut WorkersLocked<'_>, error: Error) -> Result<()> {
        unimplemented!("unexpected worker task abort {error:?}");
    }

    async fn handle_result(
        &self,
        workers: &mut WorkersLocked<'_>,
        result: WorkerResult,
    ) -> Result<()> {
        let (service, result) = result;
        match result {
            Ok(()) => self.handle_finished(workers, &service).await,
            Err(error) => self.handle_error(workers, &service, error).await,
        }
    }

    async fn handle_finished(
        &self,
        _workers: &mut WorkersLocked<'_>,
        service: &Arc<dyn Service>,
    ) -> Result<()> {
        debug!("service {:?} worker finished", service.name());
        self.set_state(service.name(), |prev| WorkerState {
            status: WorkerStatus::Finished,
            stopped_ms: Some(now_millis()),
            ..prev.cloned().unwrap_or_else(fresh)
        });
        Ok(())
    }

    async fn handle_error(
        &self,
        workers: &mut WorkersLocked<'_>,
        service: &Arc<dyn Service>,
        error: Error,
    ) -> Result<()> {
        let name = service.name();
        error!("service {name:?} aborted: {error}");
        self.set_state(name, |prev| WorkerState {
            status: WorkerStatus::Failed,
            stopped_ms: Some(now_millis()),
            error: Some(error.to_string()),
            ..prev.cloned().unwrap_or_else(fresh)
        });

        if !self.server.running() {
            debug_warn!("service {name:?} error ignored on shutdown.");
            return Ok(());
        }

        if !error.is_panic() {
            return Err(error);
        }

        let delay = Duration::from_millis(RESTART_DELAY_MS);
        warn!(
            "service {name:?} worker restarting after {} delay",
            time::pretty(delay)
        );
        sleep(delay).await;

        self.start_worker(workers, service).await
    }

    async fn start_worker(
        &self,
        workers: &mut WorkersLocked<'_>,
        service: &Arc<dyn Service>,
    ) -> Result<()> {
        if !self.server.running() {
            return Err!(
                "Service {:?} worker not starting during server shutdown.",
                service.name()
            );
        }

        debug!("Service {:?} worker starting...", service.name());
        self.set_state(service.name(), |prev| WorkerState {
            status: WorkerStatus::Running,
            started_ms: now_millis(),
            stopped_ms: None,
            restarts: prev.map_or(0, |prev| prev.restarts.saturating_add(1)),
            error: prev.and_then(|prev| prev.error.clone()),
        });
        workers.spawn_on(worker(service.clone()), self.server.runtime());

        Ok(())
    }
}

#[tracing::instrument(
	parent = None,
	level = "trace",
	skip_all,
	fields(service = %service.name()),
)]
async fn worker(service: Arc<dyn Service>) -> WorkerResult {
    let service_ = Arc::clone(&service);
    let result = AssertUnwindSafe(service_.worker())
        .catch_unwind()
        .map_err(Error::from_panic);

    let result = if service.unconstrained() {
        tokio::task::unconstrained(result).await
    } else {
        result.await
    };

    (service, result.unwrap_or_else(Err))
}

fn fresh() -> WorkerState {
    WorkerState {
        status: WorkerStatus::Running,
        started_ms: now_millis(),
        stopped_ms: None,
        restarts: 0,
        error: None,
    }
}
