//! Renderers turning a log record into something a Matrix client can display.
//!
//! These back the admin room's log capture, so the message they are handed is
//! attacker-influenced: it routinely contains room IDs, display names and
//! remote server errors. Every renderer therefore escapes the message for its
//! target syntax rather than pasting it in raw. The level and span name are
//! static strings produced by `tracing` itself and need no escaping.

use std::fmt::Write;

use super::{Level, color};
use crate::{Result, utils::HtmlEscape};

/// Renders one record as a line of HTML, as sent to a Matrix room.
pub fn html<S>(out: &mut S, level: &Level, span: &str, msg: &str) -> Result<()>
where
    S: Write + ?Sized,
{
    let color = color::code_tag(level);
    let level = level.as_str().to_uppercase();
    let span = format!("{span:^12}");

    write!(
        out,
        "<font data-mx-color=\"{color}\"><code>{level:>5}</code></font> \
         <code>{span}</code> <code>{msg}</code><br>",
        span = HtmlEscape(&span),
        msg = HtmlEscape(msg),
    )?;

    Ok(())
}

/// Renders one record as a line of Markdown.
pub fn markdown<S>(out: &mut S, level: &Level, span: &str, msg: &str) -> Result<()>
where
    S: Write + ?Sized,
{
    let level = level.as_str().to_uppercase();

    write!(out, "`{level:>5}` `{span:^12}` ")?;
    code_span(out, msg)?;
    writeln!(out)?;

    Ok(())
}

/// Renders one record as a row of the table opened by [`markdown_table_head`].
pub fn markdown_table<S>(out: &mut S, level: &Level, span: &str, msg: &str) -> Result<()>
where
    S: Write + ?Sized,
{
    let level = level.as_str().to_uppercase();

    write!(out, "| {level:>5} | {span:^12} | ")?;
    table_cell(out, msg)?;
    writeln!(out, " |")?;

    Ok(())
}

/// Writes the header the rows from [`markdown_table`] belong under.
pub fn markdown_table_head<S>(out: &mut S) -> Result<()>
where
    S: Write + ?Sized,
{
    writeln!(out, "| level | span | message |")?;
    writeln!(out, "| ------: | :-----: | :------- |")?;

    Ok(())
}

/// Writes `text` as a Markdown code span with a backtick fence long enough that
/// the text cannot close it early.
///
/// A fixed pair of backticks — what the naive rendering uses — lets any message
/// containing one escape the span and inject Markdown of its own.
fn code_span<S>(out: &mut S, text: &str) -> Result<()>
where
    S: Write + ?Sized,
{
    // An empty span is not a code span at all, so render one space instead.
    let text = if text.is_empty() { " " } else { text };

    // A span opened by N backticks is closed by the next run of exactly N, so
    // one more than the longest run inside the text is always safe.
    let fence = "`".repeat(longest_backtick_run(text).saturating_add(1));

    // A leading or trailing backtick would be absorbed into the fence; a pair
    // of spaces around the text is stripped again when the span is rendered.
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };

    write!(out, "{fence}{pad}{text}{pad}{fence}")?;

    Ok(())
}

/// Writes `text` into a Markdown table cell, neutralising the two characters
/// that would otherwise end the cell or the row.
fn table_cell<S>(out: &mut S, text: &str) -> Result<()>
where
    S: Write + ?Sized,
{
    for ch in text.chars() {
        match ch {
            '|' => out.write_str("\\|")?,
            '\r' | '\n' => out.write_char(' ')?,
            _ => out.write_char(ch)?,
        }
    }

    Ok(())
}

/// Length of the longest consecutive run of backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut longest: usize = 0;
    let mut run: usize = 0;

    for ch in text.chars() {
        run = if ch == '`' { run.saturating_add(1) } else { 0 };
        longest = longest.max(run);
    }

    longest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render<F>(fun: F, msg: &str) -> String
    where
        F: Fn(&mut String, &Level, &str, &str) -> Result<()>,
    {
        let mut out = String::new();
        fun(&mut out, &Level::INFO, "span", msg).expect("rendered");
        out
    }

    #[test]
    fn html_marks_up_level_span_and_message() {
        let out = render(html, "hello");

        assert!(out.contains("data-mx-color=\"#00FF00\""), "{out}");
        assert!(out.contains("<code> INFO</code>"), "{out}");
        assert!(out.contains("<code>    span    </code>"), "{out}");
        assert!(out.ends_with("<code>hello</code><br>"), "{out}");
    }

    #[test]
    fn html_escapes_the_message() {
        let out = render(html, "<img src=x onerror=alert(1)>");

        assert!(!out.contains("<img"), "message escaped its code tag: {out}");
        assert!(out.contains("&lt;img src=x onerror=alert(1)&gt;"), "{out}");
    }

    #[test]
    fn markdown_fences_a_message_containing_backticks() {
        let out = render(markdown, "a `code` b");

        assert!(out.ends_with("``a `code` b``\n"), "{out}");

        // The naive single-backtick rendering would have closed the span here.
        let padded = render(markdown, "` **bold** `");
        assert!(padded.contains("`` ` **bold** ` ``"), "{padded}");
    }

    #[test]
    fn markdown_renders_empty_and_plain_messages() {
        assert!(render(markdown, "").ends_with("` `\n"));
        assert!(render(markdown, "plain").ends_with("`plain`\n"));
    }

    #[test]
    fn markdown_table_row_cannot_be_broken_by_the_message() {
        let out = render(markdown_table, "a | b\nc");

        assert_eq!(out.matches('\n').count(), 1, "row spans one line: {out}");
        assert_eq!(out.matches("\\|").count(), 1, "the pipe was escaped: {out}");
        assert_eq!(
            out.matches('|').count(),
            5,
            "four cell separators plus the escaped pipe: {out}"
        );
        assert!(out.contains("a \\| b c |"), "{out}");
    }

    #[test]
    fn markdown_table_head_precedes_rows() {
        let mut out = String::new();
        markdown_table_head(&mut out).expect("rendered");
        markdown_table(&mut out, &Level::ERROR, "span", "boom").expect("rendered");

        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 3, "{out}");
        assert!(lines[1].starts_with("| ---"), "{out}");
        assert!(lines[2].contains("ERROR"), "{out}");
    }

    #[test]
    fn longest_backtick_run_counts_consecutive_ticks() {
        assert_eq!(longest_backtick_run(""), 0);
        assert_eq!(longest_backtick_run("none"), 0);
        assert_eq!(longest_backtick_run("a`b``c"), 2);
        assert_eq!(longest_backtick_run("```"), 3);
    }
}
