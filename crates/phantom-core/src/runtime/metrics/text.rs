//! The Prometheus text exposition format, version 0.0.4, written by hand.
//!
//! A metrics crate would bring a registry, a global recorder and its own
//! dependency tree for what is, on the wire, a few lines of text per series.
//! The values are kept in atomics where they are produced, and this module
//! only has to spell them out.

use std::fmt::{Display, Write};

use super::http::{BUCKETS, Snapshot};

/// The `Content-Type` a scrape is answered with.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

/// A scrape being written.
#[derive(Default)]
pub struct Text {
    out: String,
}

/// A sample's value. Integers are written exactly; a float is written the way
/// Prometheus spells the values Rust's `Display` spells differently.
pub trait Value {
    fn write(&self, out: &mut String);
}

impl Text {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a metric family: its `# HELP` and `# TYPE` lines. Every sample
    /// of the family must follow before the next one starts.
    pub fn family(&mut self, name: &str, kind: Kind, help: &str) {
        let kind = match kind {
            Kind::Counter => "counter",
            Kind::Gauge => "gauge",
            Kind::Histogram => "histogram",
        };

        let help = help.replace('\\', "\\\\").replace('\n', "\\n");
        writeln!(self.out, "# HELP {name} {help}").expect("writing to a String");
        writeln!(self.out, "# TYPE {name} {kind}").expect("writing to a String");
    }

    pub fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl Value) {
        self.out.push_str(name);
        self.labels(labels, None);
        self.out.push(' ');
        value.write(&mut self.out);
        self.out.push('\n');
    }

    /// A whole family of one sample, for the common single-valued metric.
    pub fn single(&mut self, name: &str, kind: Kind, help: &str, value: impl Value) {
        self.family(name, kind, help);
        self.sample(name, &[], value);
    }

    /// One histogram series: its buckets, `+Inf`, sum and count.
    pub fn histogram(&mut self, name: &str, labels: &[(&str, &str)], snapshot: &Snapshot) {
        let bucket = format!("{name}_bucket");
        for (bound, count) in BUCKETS.iter().zip(snapshot.cumulative) {
            self.out.push_str(&bucket);
            self.labels(labels, Some(&bound.to_string()));
            self.out.push(' ');
            count.write(&mut self.out);
            self.out.push('\n');
        }

        self.out.push_str(&bucket);
        self.labels(labels, Some("+Inf"));
        self.out.push(' ');
        snapshot.count.write(&mut self.out);
        self.out.push('\n');

        self.sample(&format!("{name}_sum"), labels, snapshot.sum_secs);
        self.sample(&format!("{name}_count"), labels, snapshot.count);
    }

    #[must_use]
    pub fn finish(self) -> String {
        self.out
    }

    fn labels(&mut self, labels: &[(&str, &str)], le: Option<&str>) {
        if labels.is_empty() && le.is_none() {
            return;
        }

        self.out.push('{');
        let mut first = true;
        for (name, value) in labels.iter().copied().chain(le.map(|le| ("le", le))) {
            if !first {
                self.out.push(',');
            }
            first = false;

            self.out.push_str(name);
            self.out.push_str("=\"");
            escape_into(&mut self.out, value);
            self.out.push('"');
        }
        self.out.push('}');
    }
}

/// A label value with `\`, `"` and newlines escaped, as the format requires.
fn escape_into(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
}

fn write_display(out: &mut String, value: impl Display) {
    write!(out, "{value}").expect("writing to a String");
}

impl Value for u64 {
    fn write(&self, out: &mut String) {
        write_display(out, self);
    }
}

impl Value for usize {
    fn write(&self, out: &mut String) {
        write_display(out, self);
    }
}

impl Value for i64 {
    fn write(&self, out: &mut String) {
        write_display(out, self);
    }
}

impl Value for u32 {
    fn write(&self, out: &mut String) {
        write_display(out, self);
    }
}

impl Value for f64 {
    fn write(&self, out: &mut String) {
        if self.is_nan() {
            out.push_str("NaN");
        } else if self.is_infinite() {
            out.push_str(if self.is_sign_positive() {
                "+Inf"
            } else {
                "-Inf"
            });
        } else {
            write_display(out, self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Text};
    use crate::runtime::metrics::http::{BUCKETS, Snapshot};

    #[test]
    fn writes_families_and_escapes_labels() {
        let mut text = Text::new();
        text.family("phantom_things", Kind::Gauge, "Things.\nMany of them.");
        text.sample("phantom_things", &[("name", "a \"quoted\"\\path\n")], 3_u64);
        text.single("phantom_up", Kind::Gauge, "Up.", 1.5_f64);

        assert_eq!(
            text.finish(),
            "# HELP phantom_things Things.\\nMany of them.\n# TYPE phantom_things gauge\n\
             phantom_things{name=\"a \\\"quoted\\\"\\\\path\\n\"} 3\n\
             # HELP phantom_up Up.\n# TYPE phantom_up gauge\nphantom_up 1.5\n"
        );
    }

    #[test]
    fn a_histogram_ends_in_inf_sum_and_count() {
        let snapshot = Snapshot {
            cumulative: [1; BUCKETS.len()],
            count: 2,
            sum_secs: 0.25,
        };

        let mut text = Text::new();
        text.histogram("h", &[("route", "/x")], &snapshot);
        let out = text.finish();

        assert!(out.starts_with("h_bucket{route=\"/x\",le=\"0.005\"} 1\n"));
        assert!(out.contains("h_bucket{route=\"/x\",le=\"+Inf\"} 2\n"));
        assert!(out.contains("h_sum{route=\"/x\"} 0.25\n"));
        assert!(out.ends_with("h_count{route=\"/x\"} 2\n"));
        assert_eq!(out.lines().count(), BUCKETS.len() + 3);
    }
}
