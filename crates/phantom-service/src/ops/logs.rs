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
    time::{SystemTime, UNIX_EPOCH},
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
    /// Counts every line kept, so a reader can ask for only the lines after
    /// the last one it saw. It starts from the startup time in microseconds
    /// rather than from one, so it keeps rising across a restart and a
    /// reader's place stays valid when the server comes back.
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
        let first_seq = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |since| {
                u64::try_from(since.as_micros()).unwrap_or(u64::MAX / 2)
            });

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
                    seq: lines
                        .back()
                        .map_or(first_seq, |last| last.seq.saturating_add(1)),
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
    /// limit. The oldest of them come first when there are more than limit,
    /// so a reader asking again from the newest it got misses none. A reader
    /// that fell more than the buffer behind gets a gap, which the jump in
    /// numbering shows; one whose place is ahead of every line kept, as after
    /// the clock was set back across a restart, gets the newest lines.
    #[must_use]
    pub fn since(&self, since: u64, limit: usize) -> Vec<Line> {
        lines_since(&self.lines.lock().expect("locked"), since, limit)
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

fn lines_since(lines: &VecDeque<Line>, since: u64, limit: usize) -> Vec<Line> {
    if lines.back().is_some_and(|newest| since > newest.seq) {
        return lines.iter().rev().take(limit).cloned().collect();
    }

    let mut after: Vec<Line> = lines
        .iter()
        .skip_while(|line| line.seq <= since)
        .take(limit)
        .cloned()
        .collect();

    after.reverse();
    after
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use phantom_core::tracing::Level;

    use super::{Line, lines_since};

    fn buffer(seqs: std::ops::RangeInclusive<u64>) -> VecDeque<Line> {
        seqs.map(|seq| Line {
            seq,
            at_ms: 0,
            level: Level::INFO,
            target: String::new(),
            span: String::new(),
            message: String::new(),
        })
        .collect()
    }

    fn seqs(lines: &[Line]) -> Vec<u64> {
        lines.iter().map(|line| line.seq).collect()
    }

    #[test]
    fn a_reader_behind_by_more_than_limit_misses_nothing() {
        let lines = buffer(101..=110);

        let first = lines_since(&lines, 100, 4);
        assert_eq!(
            seqs(&first),
            [104, 103, 102, 101],
            "the oldest after, newest first"
        );

        let second = lines_since(&lines, first[0].seq, 4);
        assert_eq!(seqs(&second), [108, 107, 106, 105]);
    }

    #[test]
    fn a_reader_up_to_date_gets_nothing() {
        assert!(lines_since(&buffer(1..=5), 5, 10).is_empty());
    }

    #[test]
    fn a_place_ahead_of_every_line_gets_the_newest() {
        let lines = buffer(1..=5);

        assert_eq!(seqs(&lines_since(&lines, 900, 2)), [5, 4]);
    }
}
