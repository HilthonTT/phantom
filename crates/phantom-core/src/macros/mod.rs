pub mod defer;
pub mod predicate;
pub mod tuple;

#[macro_export]
macro_rules! format_maybe {
    ($s:literal $(,)?) => {
        if $crate::is_format!($s) { ::std::format!($s).into() } else { $s.into() }
    };

    ($s:literal, $($args:tt)+) => {
        ::std::format!($s, $($args)+).into()
    };
}

#[macro_export]
macro_rules! is_format {
    ($s:literal) => {
        $crate::macros::has_braces($s)
    };

    ($($s:tt)+) => {
        false
    };
}

#[macro_export]
macro_rules! extract_variant {
    ( $e:expr, $( $variant:path )|* ) => {
        match $e {
            $( $variant(value) => Some(value), )*
            _ => None,
        }
    };
}

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
