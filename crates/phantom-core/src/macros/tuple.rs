//! Macros for taking tuples apart in combinator position.

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

#[cfg(test)]
mod tests {
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
