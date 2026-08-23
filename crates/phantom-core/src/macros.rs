//! Small helper macros shared across the crate.
//!
//! Error construction lives in [`crate::error::err`] instead; `err!` and `Err!`
//! come from there.

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

/// Expands to a closure testing its argument for equality with `$input`.
///
/// Handy in combinator position, e.g. `iter.filter(is_equal_to!(&'.'))`.
#[macro_export]
macro_rules! is_equal_to {
    ($input:expr) => {
        move |x| x == $input
    };
}

/// Expands to a closure testing its argument for being less than `$input`.
///
/// Handy in combinator position, e.g. `iter.skip_while(is_less_than!(&pivot))`.
#[macro_export]
macro_rules! is_less_than {
    ($input:expr) => {
        move |x| x < $input
    };
}

/// Tests `$input` for being non-zero, or expands to a closure doing so when
/// called with no argument.
#[macro_export]
macro_rules! is_nonzero {
    () => {
        |x| x != 0
    };
    ($input:expr) => {
        $input != 0
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
