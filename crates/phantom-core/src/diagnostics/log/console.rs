use std::{env, io, sync::LazyLock};

use tracing::{
    Event, Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    field::RecordFields,
    fmt,
    fmt::{
        FmtContext, FormatEvent, FormatFields, MakeWriter,
        format::{DefaultVisitor, Format, Full, Pretty, Writer},
    },
    registry::LookupSpan,
};

use crate::Config;

static SYSTEMD_MODE: LazyLock<bool> = LazyLock::new(|| {
    env::var("JOURNAL_STREAM").is_ok()
        && env::var("SYSTEMD_EXEC_PID")
            .ok()
            .and_then(|pid| pid.parse::<u32>().ok())
            .is_some_and(|pid| pid == std::process::id())
});

#[inline]
#[must_use]
pub fn is_systemd_mode() -> bool {
    *SYSTEMD_MODE
}

pub struct ConsoleWriter {
    stdout: io::Stdout,
    stderr: io::Stderr,
    use_stderr: bool,
}

impl ConsoleWriter {
    #[must_use]
    pub fn new(_config: &Config) -> Self {
        Self {
            stdout: io::stdout(),
            stderr: io::stderr(),
            use_stderr: journal_stream().is_some(),
        }
    }
}

impl<'a> MakeWriter<'a> for ConsoleWriter {
    type Writer = &'a Self;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

impl io::Write for &'_ ConsoleWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.use_stderr {
            self.stderr.lock().write(buf)
        } else {
            self.stdout.lock().write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.use_stderr {
            self.stderr.lock().flush()
        } else {
            self.stdout.lock().flush()
        }
    }
}

#[derive(Clone)]
pub struct ConsoleFormat {
    full: Format<Full>,
    pretty: Format<Pretty>,
}

impl ConsoleFormat {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let ansi = config.logging.log_colors && !is_systemd_mode();

        Self {
            full: Format::<Full>::default()
                .with_thread_ids(config.logging.log_thread_ids)
                .with_ansi(ansi),

            pretty: fmt::format()
                .pretty()
                .with_ansi(ansi)
                .with_thread_names(true)
                .with_thread_ids(true)
                .with_target(true)
                .with_file(true)
                .with_line_number(true)
                .with_source_location(true),
        }
    }
}

impl<S, N> FormatEvent<S, N> for ConsoleFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        writer: Writer<'_>,
        event: &Event<'_>,
    ) -> Result<(), std::fmt::Error> {
        let is_debug =
            cfg!(debug_assertions) && event.fields().any(|field| field.name() == "_debug");

        match *event.metadata().level() {
            Level::ERROR if !is_debug => self.pretty.format_event(ctx, writer, event),
            _ => self.full.format_event(ctx, writer, event),
        }
    }
}

impl<'writer> FormatFields<'writer> for ConsoleFormat {
    fn format_fields<R>(&self, writer: Writer<'writer>, fields: R) -> Result<(), std::fmt::Error>
    where
        R: RecordFields,
    {
        let mut visitor = ConsoleVisitor {
            visitor: DefaultVisitor::<'_>::new(writer, true),
        };

        fields.record(&mut visitor);

        Ok(())
    }
}

struct ConsoleVisitor<'a> {
    visitor: DefaultVisitor<'a>,
}

const INTERNAL_PREFIX: char = '_';

impl Visit for ConsoleVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if is_internal(field) {
            return;
        }

        self.visitor.record_debug(field, value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if is_internal(field) {
            return;
        }

        self.visitor.record_str(field, value);
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if is_internal(field) {
            return;
        }

        self.visitor.record_error(field, value);
    }
}

#[inline]
fn is_internal(field: &Field) -> bool {
    field.name().starts_with(INTERNAL_PREFIX)
}

fn journal_stream() -> Option<(u64, u64)> {
    is_systemd_mode()
        .then(|| env::var("JOURNAL_STREAM").ok())
        .flatten()
        .as_deref()
        .and_then(parse_journal_stream)
}

fn parse_journal_stream(var: &str) -> Option<(u64, u64)> {
    let (device, inode) = var.split_once(':')?;

    Some((device.parse().ok()?, inode.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_stream_parses_a_device_inode_pair() {
        assert_eq!(parse_journal_stream("8:30638"), Some((8, 30638)));
        assert_eq!(parse_journal_stream("0:0"), Some((0, 0)));
    }

    #[test]
    fn journal_stream_rejects_anything_else() {
        assert_eq!(parse_journal_stream(""), None);
        assert_eq!(parse_journal_stream("8"), None);
        assert_eq!(parse_journal_stream("8:"), None);
        assert_eq!(parse_journal_stream("eight:30638"), None);
        assert_eq!(parse_journal_stream("-1:0"), None);
    }
}
