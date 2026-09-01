use std::convert::Infallible;

use super::Result;

/// Unwraps a [`Result`] whose error cannot be constructed.
pub trait UnwrapInfallible<T> {
    fn unwrap_infallible(self) -> T;
}

impl<T> UnwrapInfallible<T> for Result<T, Infallible> {
    #[inline]
    fn unwrap_infallible(self) -> T {
        match self {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_the_only_possible_variant() {
        let result: Result<u8, Infallible> = Ok(9);

        assert_eq!(result.unwrap_infallible(), 9);
    }
}
