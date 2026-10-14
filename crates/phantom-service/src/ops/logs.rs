//! The server's recent log, kept in memory for the admin API: the last lines
//! at info level and above, from the moment the services are built.
//!
//! An admin can widen the capture to phantom's own debug lines while chasing
//! something down. Other crates' debug lines stay out even then: hyper, h2
//! and rustls between them would push everything else out of the buffer
//! within seconds. The setting is not saved; a restart goes back to info.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use phantom_core::{
    Result,
    diagnostics::log::capture::{Capture, Data, Guard},
    time::now_millis,
    tracing::Level,
};

/// How many lines are kept; the oldest go first.
const CAPACITY: usize = 1000;

pub struct Service {
    lines: Arc<Mutex<VecDeque<Line>>>,

    /// Whether phantom's own debug lines are kept as well.
    debug: Arc<AtomicBool>,

    // Captures for as long as the service lives.
    _capture: Guard,
}

#[derive(Clone, Debug)]
pub struct Line {
    /// Counts every line kept since startup, so a reader can ask for only
    /// the lines after the last one it saw.
    pub seq: u64,
    pub at_ms: u64,
    pub level: Level,
    pub target: String,
    pub span: String,
    pub message: String,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let lines = Arc::new(Mutex::new(VecDeque::<Line>::with_capacity(CAPACITY)));
        let sink = lines.clone();
        let debug = Arc::new(AtomicBool::new(false));
        let wanted = debug.clone();

        let capture = Capture::new(
            &args.server.log.capture,
            Some(move |data: Data<'_>| {
                data.level() <= Level::INFO
                    || (data.level() == Level::DEBUG
                        && data.our_modules()
                        && wanted.load(Ordering::Relaxed))
            }),
            move |data: Data<'_>| {
                let mut lines = sink.lock().expect("locked");
                let line = Line {
                    seq: lines.back().map_or(1, |last| last.seq.saturating_add(1)),
                    at_ms: now_millis(),
                    level: data.level(),
                    target: data.mod_name().to_owned(),
                    span: data.span_name().to_owned(),
                    message: data.message().to_owned(),
                };

                if lines.len() == CAPACITY {
                    lines.pop_front();
                }
                lines.push_back(line);
            },
        );

        Ok(Arc::new(Self {
            lines,
            debug,
            _capture: capture.start(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// The lines kept, newest first, up to limit.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<Line> {
        self.lines
            .lock()
            .expect("locked")
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// The lines kept after the one numbered since, newest first, up to
    /// limit. A reader that fell more than the buffer behind gets the newest
    /// lines and a gap, which the jump in numbering shows.
    #[must_use]
    pub fn since(&self, since: u64, limit: usize) -> Vec<Line> {
        self.lines
            .lock()
            .expect("locked")
            .iter()
            .rev()
            .take_while(|line| line.seq > since)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Whether phantom's own debug lines are kept as well as info and above.
    #[must_use]
    pub fn captures_debug(&self) -> bool {
        self.debug.load(Ordering::Relaxed)
    }

    pub fn capture_debug(&self, debug: bool) {
        self.debug.store(debug, Ordering::Relaxed);
    }
}
