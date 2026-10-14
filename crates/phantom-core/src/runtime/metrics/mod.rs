//! What the server counts about itself: tokio's runtime metrics, and the
//! counters the Prometheus endpoint exposes. The counters live here, on the
//! [`Server`](super::server::Server) every service already holds, so a service
//! records into a plain atomic without depending on whoever serves them.

pub mod http;
pub mod text;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use tokio::runtime;
use tokio_metrics::TaskMonitor;
#[cfg(tokio_unstable)]
use tokio_metrics::{RuntimeIntervals, RuntimeMonitor};

pub struct Metrics {
    _runtime: Option<runtime::Handle>,

    runtime_metrics: Option<runtime::RuntimeMetrics>,

    task_monitor: Option<TaskMonitor>,

    #[cfg(tokio_unstable)]
    _runtime_monitor: Option<RuntimeMonitor>,

    #[cfg(tokio_unstable)]
    runtime_intervals: std::sync::Mutex<Option<RuntimeIntervals>>,

    pub requests_handle_active: AtomicU32,
    pub requests_handle_finished: AtomicU32,
    pub requests_panic: AtomicU32,

    /// Requests by route template, method and status class.
    pub http: http::Http,

    /// `/sync` and `/events` requests parked waiting for something to happen.
    pub sync_long_polls: Gauge,

    /// Outgoing transactions answered, and failed, by destination class.
    pub sending_sent: [AtomicU64; SendClass::ALL.len()],
    pub sending_failed: [AtomicU64; SendClass::ALL.len()],

    /// Federation transactions received, and what became of their contents.
    pub federation_transactions: AtomicU64,
    pub federation_pdus_accepted: AtomicU64,
    pub federation_pdus_rejected: AtomicU64,
    pub federation_edus: AtomicU64,
}

/// Where an outgoing transaction goes, as a label: one series per class
/// rather than per destination, which would grow with the federation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendClass {
    Federation,
    Appservice,
    Push,
}

/// A count of things in progress, held up by a [`GaugeGuard`] for as long as
/// each one is.
#[derive(Default)]
pub struct Gauge(AtomicU64);

/// Counts one thing in progress until dropped, however its future ends.
pub struct GaugeGuard<'a>(&'a Gauge);

impl Metrics {
    #[must_use]
    pub fn new(runtime: Option<runtime::Handle>) -> Self {
        #[cfg(tokio_unstable)]
        let runtime_monitor = runtime.as_ref().map(RuntimeMonitor::new);

        #[cfg(tokio_unstable)]
        let runtime_intervals = runtime_monitor.as_ref().map(RuntimeMonitor::intervals);

        Self {
            _runtime: runtime.clone(),

            runtime_metrics: runtime.as_ref().map(runtime::Handle::metrics),

            task_monitor: runtime.map(|_| TaskMonitor::new()),

            #[cfg(tokio_unstable)]
            _runtime_monitor: runtime_monitor,

            #[cfg(tokio_unstable)]
            runtime_intervals: std::sync::Mutex::new(runtime_intervals),

            requests_handle_active: AtomicU32::new(0),
            requests_handle_finished: AtomicU32::new(0),
            requests_panic: AtomicU32::new(0),

            http: http::Http::default(),
            sync_long_polls: Gauge::default(),
            sending_sent: Default::default(),
            sending_failed: Default::default(),
            federation_transactions: AtomicU64::new(0),
            federation_pdus_accepted: AtomicU64::new(0),
            federation_pdus_rejected: AtomicU64::new(0),
            federation_edus: AtomicU64::new(0),
        }
    }

    #[cfg(tokio_unstable)]
    pub fn runtime_interval(&self) -> Option<tokio_metrics::RuntimeMetrics> {
        self.runtime_intervals
            .lock()
            .expect("locked")
            .as_mut()
            .map(Iterator::next)
            .expect("next interval")
    }

    #[inline]
    pub fn task_root(&self) -> Option<&TaskMonitor> {
        self.task_monitor.as_ref()
    }

    #[inline]
    pub fn num_workers(&self) -> usize {
        self.runtime_metrics()
            .map_or(0, runtime::RuntimeMetrics::num_workers)
    }

    #[inline]
    pub fn runtime_metrics(&self) -> Option<&runtime::RuntimeMetrics> {
        self.runtime_metrics.as_ref()
    }

    /// An outgoing transaction to a destination of `class` was answered, or
    /// failed.
    pub fn sent(&self, class: SendClass, ok: bool) {
        let counters = if ok {
            &self.sending_sent
        } else {
            &self.sending_failed
        };

        counters[class.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// Writes every metric kept here: the HTTP histograms, the counters, and
    /// tokio's stable runtime metrics when there is a runtime to read.
    pub fn encode(&self, out: &mut text::Text) {
        use text::Kind::{Counter, Gauge as GaugeKind, Histogram};

        out.family(
            "phantom_http_requests_duration_seconds",
            Histogram,
            "Time to answer HTTP requests, by route template, method and status class.",
        );
        for (labels, snapshot) in self.http.snapshot() {
            out.histogram(
                "phantom_http_requests_duration_seconds",
                &[
                    ("route", &labels.route),
                    ("method", labels.method),
                    ("status", labels.class),
                ],
                &snapshot,
            );
        }

        out.single(
            "phantom_http_requests_in_flight",
            GaugeKind,
            "HTTP requests being answered.",
            self.requests_handle_active.load(Ordering::Relaxed),
        );

        out.single(
            "phantom_sync_long_polls",
            GaugeKind,
            "Sync requests parked waiting for something to happen.",
            self.sync_long_polls.get(),
        );

        out.family(
            "phantom_sending_transactions_total",
            Counter,
            "Outgoing transactions by destination class and outcome.",
        );
        for class in SendClass::ALL {
            for (outcome, counters) in [
                ("sent", &self.sending_sent),
                ("failed", &self.sending_failed),
            ] {
                out.sample(
                    "phantom_sending_transactions_total",
                    &[("class", class.as_str()), ("outcome", outcome)],
                    counters[class.index()].load(Ordering::Relaxed),
                );
            }
        }

        out.single(
            "phantom_federation_transactions_received_total",
            Counter,
            "Federation transactions received from other servers.",
            self.federation_transactions.load(Ordering::Relaxed),
        );

        out.family(
            "phantom_federation_pdus_received_total",
            Counter,
            "PDUs received over federation, by whether they were accepted.",
        );
        for (outcome, counter) in [
            ("accepted", &self.federation_pdus_accepted),
            ("rejected", &self.federation_pdus_rejected),
        ] {
            out.sample(
                "phantom_federation_pdus_received_total",
                &[("outcome", outcome)],
                counter.load(Ordering::Relaxed),
            );
        }

        out.single(
            "phantom_federation_edus_received_total",
            Counter,
            "EDUs received over federation.",
            self.federation_edus.load(Ordering::Relaxed),
        );

        if let Some(runtime) = self.runtime_metrics() {
            encode_runtime(runtime, out);
        }
    }
}

fn encode_runtime(runtime: &runtime::RuntimeMetrics, out: &mut text::Text) {
    use text::Kind::{Counter, Gauge};

    out.single(
        "tokio_workers",
        Gauge,
        "Worker threads of the tokio runtime.",
        runtime.num_workers(),
    );
    out.single(
        "tokio_alive_tasks",
        Gauge,
        "Tasks alive on the tokio runtime.",
        runtime.num_alive_tasks(),
    );
    out.single(
        "tokio_global_queue_depth",
        Gauge,
        "Tasks waiting in the runtime's global queue.",
        runtime.global_queue_depth(),
    );

    #[cfg(target_has_atomic = "64")]
    {
        let workers = 0..runtime.num_workers();
        let busy: f64 = workers
            .clone()
            .map(|worker| runtime.worker_total_busy_duration(worker).as_secs_f64())
            .sum();
        let parks: u64 = workers
            .map(|worker| runtime.worker_park_count(worker))
            .sum();

        out.single(
            "tokio_workers_busy_seconds_total",
            Counter,
            "Time the runtime's workers spent busy, summed over the workers.",
            busy,
        );
        out.single(
            "tokio_workers_park_total",
            Counter,
            "Times the runtime's workers parked for lack of work.",
            parks,
        );
    }
}

impl SendClass {
    pub const ALL: [Self; 3] = [Self::Federation, Self::Appservice, Self::Push];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Federation => "federation",
            Self::Appservice => "appservice",
            Self::Push => "push",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Federation => 0,
            Self::Appservice => 1,
            Self::Push => 2,
        }
    }
}

impl Gauge {
    #[must_use]
    pub fn enter(&self) -> GaugeGuard<'_> {
        self.0.fetch_add(1, Ordering::Relaxed);

        GaugeGuard(self)
    }

    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Drop for GaugeGuard<'_> {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::Relaxed);
    }
}
