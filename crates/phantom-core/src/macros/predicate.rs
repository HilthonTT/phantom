//! Macros expanding to the small closures that combinators take.
//!
//! `iter.filter(is_equal_to!(&'.'))` says what it tests at a glance where the
//! closure would say it in punctuation.

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

/// Expands to a closure testing its argument with [`matches!`].
///
/// Handy in combinator position, e.g. `.is_some_and(is_matching!('A'..='Z'))`.
#[macro_export]
macro_rules! is_matching {
    ($($pat:tt)+) => {
        |x| ::std::matches!(x, $($pat)+)
    };
}

/// Expands to a closure testing its argument for being zero.
#[macro_export]
macro_rules! is_zero {
    () => {
        $crate::is_matching!(0)
    };
}

/// Expands to a closure testing its argument for being non-empty.
#[macro_export]
macro_rules! is_not_empty {
    () => {
        |x| !x.is_empty()
    };
}

/// Expands to a closure returning its `bool` argument unchanged.
///
/// The identity predicate, for combinators that demand one.
#[macro_export]
macro_rules! is_true {
    () => {
        |x| x
    };
}

/// Expands to a closure negating its `bool` argument.
#[macro_export]
macro_rules! is_false {
    () => {
        |x| !x
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn predicates_read_as_combinator_arguments() {
        let one = 1;

        assert!(Some(2).is_some_and(is_equal_to!(2)));
        assert!(Some(1).is_some_and(is_less_than!(2)));
        assert!(is_nonzero!(one));
        assert!(Some(0).is_some_and(is_zero!()));
        assert!(Some('C').is_some_and(is_matching!('A'..='Z')));
        assert!(Some("x").is_some_and(is_not_empty!()));
        assert!(Some(true).is_some_and(is_true!()));
        assert!(Some(false).is_some_and(is_false!()));
    }
}
