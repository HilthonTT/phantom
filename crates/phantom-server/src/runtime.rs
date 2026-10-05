use std::{
    iter::once,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use phantom_core::{
    Result, debug, implement,
    runtime::sys::compute::{nth_core_available, set_affinity},
};
pub(crate) use tokio::runtime::Handle;
#[cfg(tokio_unstable)]
use tokio::runtime::HistogramConfiguration;
use tokio::runtime::{Builder, Runtime as Tokio};

use crate::args::Args;

pub(crate) struct Runtime {
    runtime: Option<Tokio>,
}

#[derive(Default)]
struct State {
    worker_affinity: bool,
    cores_occupied: AtomicUsize,
    thread_spawns: AtomicUsize,
}

const WORKER_THREAD_NAME: &str = "phantom:worker";
const WORKER_THREAD_MIN: usize = 2;
const BLOCKING_THREAD_KEEPALIVE: u64 = 36;
const BLOCKING_THREAD_NAME: &str = "phantom:spawned";
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[implement(Runtime)]
pub(crate) fn new(args: &Args) -> Result<Self> {
    let state = Arc::new(State {
        worker_affinity: args.worker_affinity,
        ..Default::default()
    });

    let mut builder = Builder::new_multi_thread();
    builder
        .enable_io()
        .enable_time()
        .worker_threads(args.worker_threads.max(WORKER_THREAD_MIN))
        .thread_keep_alive(Duration::from_secs(BLOCKING_THREAD_KEEPALIVE))
        .global_queue_interval(args.global_event_interval)
        .event_interval(args.kernel_event_interval)
        .max_io_events_per_tick(args.kernel_events_per_tick);

    state.enable_hooks(&mut builder);

    #[cfg(tokio_unstable)]
    enable_poll_histogram(&mut builder, args);

    Ok(Self {
        runtime: Some(builder.build()?),
    })
}

impl Drop for Runtime {
    #[tracing::instrument(name = "stop", level = "info", skip_all)]
    fn drop(&mut self) {
        debug!(
            timeout = ?RUNTIME_SHUTDOWN_TIMEOUT,
            "Waiting for runtime..."
        );

        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);
        }
    }
}

#[implement(Runtime)]
#[inline]
pub(crate) fn block_on<F: Future>(&self, future: F) -> F::Output {
    self.runtime().block_on(future)
}

#[implement(Runtime)]
#[inline]
pub(crate) fn handle(&self) -> &Handle {
    self.runtime().handle()
}

#[implement(Runtime)]
#[inline]
fn runtime(&self) -> &Tokio {
    self.runtime.as_ref().expect("Runtime must be initialized")
}

#[cfg(tokio_unstable)]
fn enable_poll_histogram(builder: &mut Builder, args: &Args) {
    let linear = HistogramConfiguration::linear(
        Duration::from_micros(args.worker_poll_histogram_interval),
        args.worker_poll_histogram_buckets,
    );

    builder
        .enable_metrics_poll_time_histogram()
        .metrics_poll_time_histogram_configuration(linear);
}

#[implement(State)]
fn enable_hooks(self: &Arc<Self>, builder: &mut Builder) {
    {
        let state = self.clone();
        builder.thread_name_fn(move || state.thread_name())
    };
    {
        let state = self.clone();
        builder.on_thread_start(move || state.thread_start())
    };
}

#[implement(State)]
fn thread_name(&self) -> String {
    let handle = Handle::current();
    let num_workers = handle.metrics().num_workers();
    let i = self.thread_spawns.load(Ordering::Acquire);

    if i >= num_workers {
        BLOCKING_THREAD_NAME.into()
    } else {
        WORKER_THREAD_NAME.into()
    }
}

#[implement(State)]
#[tracing::instrument(
    name = "fork",
    level = "debug",
    skip_all,
    fields(
        tid = ?thread::current().id(),
        name = %thread::current().name().unwrap_or("None"),
    ),
)]
fn thread_start(&self) {
    debug_assert!(
        thread::current().name() == Some(WORKER_THREAD_NAME)
            || thread::current().name() == Some(BLOCKING_THREAD_NAME),
        "tokio worker name mismatch at thread start"
    );

    if self.worker_affinity {
        self.set_worker_affinity();
    }

    self.thread_spawns.fetch_add(1, Ordering::AcqRel);
}

/// Pins each worker to its own core, in order, until the workers or the cores
/// run out; blocking threads are left to the scheduler.
#[implement(State)]
fn set_worker_affinity(&self) {
    let handle = Handle::current();
    let num_workers = handle.metrics().num_workers();
    let i = self.cores_occupied.fetch_add(1, Ordering::AcqRel);
    if i >= num_workers {
        return;
    }

    let Some(id) = nth_core_available(i) else {
        return;
    };

    set_affinity(once(id));
}
