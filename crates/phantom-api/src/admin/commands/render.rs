//! Turns the admin API's JSON into the notices the admin room shows.

use std::time::{Duration, UNIX_EPOCH};

use phantom_core::{html, time};
use phantom_service::ops::admin::CommandOutput;
use serde_json::Value;

/// A table column: its heading and the JSON field it shows.
pub(super) struct Column {
    title: &'static str,
    key: &'static str,

    /// The field is milliseconds since the epoch; show it as a UTC time.
    time: bool,
}

impl Column {
    pub(super) const fn new(title: &'static str, key: &'static str) -> Self {
        Self {
            title,
            key,
            time: false,
        }
    }

    pub(super) const fn time(title: &'static str, key: &'static str) -> Self {
        Self {
            title,
            key,
            time: true,
        }
    }

    fn cell(&self, row: &Value) -> String {
        let value = &row[self.key];
        match (self.time, value.as_u64()) {
            (true, Some(ms)) => timestamp(ms),
            _ => scalar(value),
        }
    }
}

/// An array of objects as an aligned table, one row each.
pub(super) fn table(rows: &Value, columns: &[Column]) -> CommandOutput {
    let rows: &[Value] = rows.as_array().map_or(&[], Vec::as_slice);
    if rows.is_empty() {
        return done("Nothing to show.");
    }

    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| columns.iter().map(|column| column.cell(row)).collect())
        .collect();

    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            cells
                .iter()
                .map(|row| row[i].chars().count())
                .chain([column.title.len()])
                .max()
                .unwrap_or(0)
        })
        .collect();

    let line = |cells: Vec<&str>| {
        cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };

    let mut text = line(columns.iter().map(|column| column.title).collect());
    for row in &cells {
        text.push('\n');
        text.push_str(&line(row.iter().map(String::as_str).collect()));
    }

    let count = rows.len();
    text.push_str(&format!(
        "\n\n{count} {}",
        if count == 1 { "row" } else { "rows" }
    ));

    preformatted(&text)
}

/// One object as a list of its fields.
pub(super) fn fields(object: &Value) -> CommandOutput {
    let Some(object) = object.as_object() else {
        return preformatted(&scalar(object));
    };

    let width = object.keys().map(String::len).max().unwrap_or(0);
    let text = object
        .iter()
        .map(|(key, value)| {
            let value = match (key.ends_with("_ms"), value.as_u64()) {
                (true, Some(ms)) => timestamp(ms),
                _ => scalar(value),
            };

            format!("{key:<width$}  {value}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    preformatted(&text)
}

/// A one-line confirmation.
pub(super) fn done(message: &str) -> CommandOutput {
    CommandOutput::notice_plain(message)
}

/// A failed command's message.
pub(super) fn error(message: &str) -> CommandOutput {
    CommandOutput::notice_plain(format!("Command failed: {message}"))
}

/// Text shown in a fixed-width block, as tables and clap's help are.
pub(super) fn preformatted(text: &str) -> CommandOutput {
    let text = text.trim_end();
    CommandOutput::notice_html(
        text,
        format!("<pre><code>{}</code></pre>", html::escape(text)),
    )
}

/// A JSON value as a table cell: strings bare, nothing as a dash.
pub(super) fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "-".to_owned(),
        Value::String(string) => string.clone(),
        Value::Array(items) if items.is_empty() => "-".to_owned(),
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn timestamp(ms: u64) -> String {
    time::format(
        UNIX_EPOCH + Duration::from_millis(ms),
        "%Y-%m-%d %H:%M:%S UTC",
    )
}
