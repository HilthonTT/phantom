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

/// Expands to a closure taking field `$idx` of its argument, e.g.
/// `.map(at!(0))` over an iterator of tuples.
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! at {
    ($idx:tt) => {
        |t| t.$idx
    };
}

/// [`at!`] borrowing the field rather than moving it.
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! ref_at {
    ($idx:tt) => {
        |ref t| &t.$idx
    };
}

/// [`at!`] over a reference to the tuple, copying the field out.
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! val_at {
    ($idx:tt) => {
        |&t| t.$idx
    };
}

/// [`at!`] dereferencing the field it takes.
#[macro_export]
#[collapse_debuginfo(yes)]
macro_rules! deref_at {
    ($idx:tt) => {
        |t| *t.$idx
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

/// Expands to a pair of the given type or of the given value, so a symmetric
/// pair need not name its half twice.
#[macro_export]
macro_rules! pair_of {
    ($decl:ty) => {
        ($decl, $decl)
    };

    ($init:expr) => {
        ($init, $init)
    };
}

/// Expands to a closure applying a function to every element of a tuple of
/// `$n` elements, e.g. `.map(apply!(2, str::parse))` over a pair of strings.
#[macro_export]
macro_rules! apply {
    (1, $($f:tt)+) => {
        |t| (($($f)+)(t.0),)
    };

    (2, $($f:tt)+) => {
        |t| (($($f)+)(t.0), ($($f)+)(t.1))
    };

    (3, $($f:tt)+) => {
        |t| (($($f)+)(t.0), ($($f)+)(t.1), ($($f)+)(t.2))
    };

    (4, $($f:tt)+) => {
        |t| (($($f)+)(t.0), ($($f)+)(t.1), ($($f)+)(t.2), ($($f)+)(t.3))
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

    #[test]
    fn accessors_pick_a_tuple_field() {
        let pairs = [(1, 'a'), (2, 'b')];

        let firsts: Vec<_> = pairs.into_iter().map(at!(0)).collect();
        assert_eq!(firsts, [1, 2]);

        let seconds: Vec<_> = pairs.iter().map(val_at!(1)).collect();
        assert_eq!(seconds, ['a', 'b']);

        let borrowed: Vec<&i32> = pairs.iter().map(ref_at!(0)).collect();
        assert_eq!(borrowed, [&1, &2]);

        let refs = [(&1, 'a')];
        let derefed: Vec<i32> = refs.into_iter().map(deref_at!(0)).collect();
        assert_eq!(derefed, [1]);
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

    #[test]
    fn pair_of_repeats_a_type_or_a_value() {
        let pair: pair_of!(u8) = pair_of!(3);

        assert_eq!(pair, (3, 3));
    }

    #[test]
    fn apply_maps_every_element_of_a_tuple() {
        let pairs: Vec<_> = [("a", "b")]
            .into_iter()
            .map(apply!(2, str::to_owned))
            .collect();
        assert_eq!(pairs, [("a".to_owned(), "b".to_owned())]);

        // The reference implementation's four-element arm applied `$f + 4` to
        // the last element, which only compiled for a numeric `$f`.
        let quads: Vec<_> = [("1", "2", "3", "4")]
            .into_iter()
            .map(apply!(4, str::parse::<u8>))
            .collect();
        assert_eq!(
            quads,
            [(Ok(1), Ok(2), Ok(3), Ok(4))],
            "every element is converted, including the last"
        );
    }
}
