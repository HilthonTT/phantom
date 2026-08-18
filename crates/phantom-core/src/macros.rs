//! Small helper macros shared across the crate.

/// Builds an [`Error`][crate::Error] from `format!` arguments.
#[macro_export]
macro_rules! err {
    ($($arg:tt)+) => {
        $crate::Error::Msg(::std::format!($($arg)+))
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
