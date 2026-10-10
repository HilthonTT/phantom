//! The server's recent log, kept in memory for the admin API: the last lines
//! at info level and above, from the moment the services are built.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
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

    // Captures for as long as the service lives.
    _capture: Guard,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub at_ms: u64,
    pub level: Level,
    pub target: String,
    pub span: String,
    pub message: String,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let lines = Arc::new(Mutex::new(VecDeque::with_capacity(CAPACITY)));
        let sink = lines.clone();

        let capture = Capture::new(
            &args.server.log.capture,
            Some(|data: Data<'_>| data.level() <= Level::INFO),
            move |data: Data<'_>| {
                let line = Line {
                    at_ms: now_millis(),
                    level: data.level(),
                    target: data.mod_name().to_owned(),
                    span: data.span_name().to_owned(),
                    message: data.message().to_owned(),
                };

                let mut lines = sink.lock().expect("locked");
                if lines.len() == CAPACITY {
                    lines.pop_front();
                }
                lines.push_back(line);
            },
        );

        Ok(Arc::new(Self {
            lines,
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
}
