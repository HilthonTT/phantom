//! Request counts and latencies, by route template, method and status class.
//!
//! The route is the template a request matched (`/_matrix/client/v3/rooms/{room_id}/send/...`),
//! never the raw path: every room ID or event ID in a path would otherwise
//! become a series of its own, and a scraper holds every series it has ever
//! seen. For the same reason a method outside the standard set is counted as
//! `OTHER` and a status by its class alone, so the number of series is bounded
//! by the routes the server registers, whatever clients send.

use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

/// The upper bounds of the latency buckets, in seconds. The long tail is there
/// for `/sync`, whose long-polls legitimately take as long as a client asks.
pub const BUCKETS: [f64; 13] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// The route label of a request no route matched.
pub const UNMATCHED: &str = "unmatched";

type Series = HashMap<(&'static str, &'static str), Arc<Histogram>>;

#[derive(Default)]
pub struct Http {
    /// Keyed by route first, so recording a request borrows the route rather
    /// than allocating a key for it; a route is only copied the first time it
    /// is seen.
    routes: RwLock<HashMap<String, Series>>,
}

/// One series' latency histogram. The buckets are stored per bucket rather
/// than cumulatively, so an observation touches one counter; the exposition
/// sums them.
pub struct Histogram {
    buckets: [AtomicU64; BUCKETS.len()],
    count: AtomicU64,
    sum_micros: AtomicU64,
}

/// A histogram's values read at one moment, cumulative as Prometheus wants
/// them.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub cumulative: [u64; BUCKETS.len()],
    pub count: u64,
    pub sum_secs: f64,
}

/// One series' labels.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Labels {
    pub route: String,
    pub method: &'static str,
    pub class: &'static str,
}

impl Http {
    pub fn observe(&self, route: &str, method: &str, status: u16, elapsed: Duration) {
        let key = (method_label(method), status_class(status));

        let found = self
            .routes
            .read()
            .expect("locked")
            .get(route)
            .and_then(|series| series.get(&key))
            .cloned();

        let histogram = found.unwrap_or_else(|| {
            self.routes
                .write()
                .expect("locked")
                .entry(route.to_owned())
                .or_default()
                .entry(key)
                .or_insert_with(|| Arc::new(Histogram::default()))
                .clone()
        });

        histogram.observe(elapsed);
    }

    /// Every series, ordered by its labels so a scrape reads stably.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(Labels, Snapshot)> {
        let routes = self.routes.read().expect("locked");
        let mut out: Vec<_> = routes
            .iter()
            .flat_map(|(route, series)| {
                series.iter().map(|(&(method, class), histogram)| {
                    let labels = Labels {
                        route: route.clone(),
                        method,
                        class,
                    };

                    (labels, histogram.snapshot())
                })
            })
            .collect();

        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum_micros: AtomicU64::new(0),
        }
    }
}

impl Histogram {
    pub fn observe(&self, elapsed: Duration) {
        let secs = elapsed.as_secs_f64();
        if let Some(bucket) = BUCKETS.iter().position(|&bound| secs <= bound) {
            self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        }

        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.sum_micros.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let mut cumulative = [0; BUCKETS.len()];
        let mut running = 0_u64;
        for (out, bucket) in cumulative.iter_mut().zip(&self.buckets) {
            running = running.saturating_add(bucket.load(Ordering::Relaxed));
            *out = running;
        }

        #[allow(clippy::as_conversions, clippy::cast_precision_loss)]
        let sum_secs = self.sum_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;

        Snapshot {
            cumulative,
            // Read after the buckets, so a request recorded in between can
            // only make the count larger than the last bucket, never smaller,
            // which is what `+Inf` being the count requires.
            count: self.count.load(Ordering::Relaxed).max(running),
            sum_secs,
        }
    }
}

/// The method as a label: one of the standard methods, or `OTHER`.
#[must_use]
pub fn method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        "PATCH" => "PATCH",
        _ => "OTHER",
    }
}

/// The status as a label: its class, `2xx` through `5xx`, or `other`.
#[must_use]
pub fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{BUCKETS, Http, method_label, status_class};

    #[test]
    fn labels_are_bounded() {
        assert_eq!(method_label("GET"), "GET");
        assert_eq!(method_label("BREW"), "OTHER");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(429), "4xx");
        assert_eq!(status_class(999), "other");
    }

    #[test]
    fn observations_land_in_cumulative_buckets() {
        let http = Http::default();
        let route = "/_matrix/client/v3/rooms/{room_id}/send/{event_type}/{txn_id}";

        http.observe(route, "PUT", 200, Duration::from_millis(3));
        http.observe(route, "PUT", 201, Duration::from_millis(200));
        http.observe(route, "PUT", 500, Duration::from_secs(1));
        http.observe(route, "PUT", 200, Duration::from_secs(120));

        let series = http.snapshot();
        assert_eq!(series.len(), 2, "one series per status class");

        let (labels, ok) = &series[0];
        assert_eq!(labels.class, "2xx");
        assert_eq!(ok.count, 3);
        assert_eq!(ok.cumulative[0], 1, "3ms is under the first bound");
        assert_eq!(ok.cumulative[5], 2, "200ms is under the 250ms bound");
        assert_eq!(
            ok.cumulative[BUCKETS.len() - 1],
            2,
            "two minutes is past every bound, so only +Inf counts it"
        );
        assert!((ok.sum_secs - 120.203).abs() < 1e-6);

        let (labels, failed) = &series[1];
        assert_eq!(labels.class, "5xx");
        assert_eq!(failed.count, 1);
    }
}
