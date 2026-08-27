//! Small declarative macros shared across the crate.
//!
//! They are grouped by what they produce: [`predicate`] for the closures that
//! read as combinator arguments, [`tuple`] for the ones that pick a field
//! apart, and [`defer`] for scope-exit bookkeeping. Everything here is
//! `#[macro_export]`ed, so the module path matters only for finding the
//! definition — callers spell them as `phantom_core::at!` and friends.
//!
//! Error construction lives in [`crate::error::construct`] instead; `err!` and
//! `Err!` come from there.

pub mod defer;
pub mod predicate;
pub mod tuple;

/// Formats `$s` only when it actually looks like a format string, so callers
/// can pass either a plain literal or a format string without paying for a
/// `format!` on the former.
#[macro_export]
macro_rules! format_maybe {
    ($s:literal $(,)?) => {
        if $crate::is_format!($s) { ::std::format!($s).into() } else { $s.into() }
    };

    ($s:literal, $($args:tt)+) => {
        ::std::format!($s, $($args)+).into()
    };
}

/// Const expression deciding whether a literal is a format string.
#[macro_export]
macro_rules! is_format {
    ($s:literal) => {
        $crate::macros::has_braces($s)
    };

    ($($s:tt)+) => {
        false
    };
}

/// Expands to `Some(value)` for the named enum variants and `None` for
/// anything else, e.g. `extract_variant!(event, Event::Left | Event::Right)`.
#[macro_export]
macro_rules! extract_variant {
    ( $e:expr, $( $variant:path )|* ) => {
        match $e {
            $( $variant(value) => Some(value), )*
            _ => None,
        }
    };
}

/// Backs [`is_format!`]. A `const fn` rather than a `const_str::contains!` so
/// the crate does not need a proc-macro dependency for this one check.
#[must_use]
pub const fn has_braces(s: &str) -> bool {
    let bytes = s.as_bytes();
    let (mut open, mut close) = (false, false);

    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => open = true,
            b'}' => close = true,
            _ => {}
        }
        i = i.saturating_add(1);
    }

    open && close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_braces_detects_a_format_string() {
        assert!(has_braces("{x}"));
        assert!(!has_braces("plain"));
        assert!(!has_braces("{ unclosed"));
    }

    #[test]
    fn extract_variant_matches_any_of_the_named_variants() {
        #[derive(Debug)]
        enum Value {
            Int(u8),
            Float(f32),
            Nothing,
        }

        assert_eq!(extract_variant!(Value::Int(3), Value::Int), Some(3));
        assert_eq!(extract_variant!(Value::Float(1.5), Value::Float), Some(1.5));
        assert_eq!(extract_variant!(Value::Float(1.5), Value::Int), None);
        assert_eq!(extract_variant!(Value::Nothing, Value::Int), None);
    }
}
