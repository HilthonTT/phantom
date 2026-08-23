//! Level colours shared by every log renderer.
//!
//! Separate from [`super::fmt`] so the admin room's HTML, its Markdown, and any
//! future renderer agree on what a level looks like.

use super::Level;

/// Foreground and background for a level, as HTML colour literals.
#[inline]
#[must_use]
pub fn html(level: &Level) -> (&'static str, &'static str) {
    match *level {
        Level::TRACE => ("#000000", "#A0A0A0"),
        Level::DEBUG => ("#000000", "#FFFFFF"),
        Level::INFO => ("#FFFFFF", "#008E00"),
        Level::WARN => ("#000000", "#FFFF00"),
        Level::ERROR => ("#000000", "#FF0000"),
    }
}

/// Foreground for a level rendered inside a `<code>` tag, which sits on the
/// client's own background rather than one we set.
#[inline]
#[must_use]
pub fn code_tag(level: &Level) -> &'static str {
    match *level {
        Level::TRACE => "#888888",
        Level::DEBUG => "#C8C8C8",
        Level::INFO => "#00FF00",
        Level::WARN => "#FFFF00",
        Level::ERROR => "#FF0000",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: [Level; 5] = [
        Level::TRACE,
        Level::DEBUG,
        Level::INFO,
        Level::WARN,
        Level::ERROR,
    ];

    #[test]
    fn every_level_maps_to_a_hex_literal() {
        for level in LEVELS {
            let (fg, bg) = html(&level);
            for color in [fg, bg, code_tag(&level)] {
                assert_eq!(color.len(), 7, "{level}: {color}");
                assert!(color.starts_with('#'), "{level}: {color}");
                assert!(
                    color[1..].bytes().all(|b| b.is_ascii_hexdigit()),
                    "{level}: {color}"
                );
            }
        }
    }

    #[test]
    fn levels_are_visually_distinct() {
        let mut tags: Vec<_> = LEVELS.iter().map(code_tag).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), LEVELS.len(), "two levels share a colour");
    }
}
