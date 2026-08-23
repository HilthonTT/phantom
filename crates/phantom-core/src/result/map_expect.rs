use std::fmt::Debug;

use super::Result;

/// Calls `expect(msg)` on a [`Result`] nested inside another container, without
/// a closure at the callsite.
pub trait MapExpect<'a, T> {
    /// Unwraps the inner [`Result`], panicking with `msg` if it is an `Err`.
    fn map_expect(self, msg: &'a str) -> T;
}

impl<'a, T, E: Debug> MapExpect<'a, Option<T>> for Option<Result<T, E>> {
    #[inline]
    fn map_expect(self, msg: &'a str) -> Option<T> {
        self.map(|result| result.expect(msg))
    }
}

impl<'a, T, E: Debug> MapExpect<'a, Result<T, E>> for Result<Option<T>, E> {
    #[inline]
    fn map_expect(self, msg: &'a str) -> Result<T, E> {
        self.map(|option| option.expect(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_a_result_inside_an_option() {
        let value: Option<Result<u8, String>> = Some(Ok(3));
        assert_eq!(value.map_expect("present"), Some(3));

        let missing: Option<Result<u8, String>> = None;
        assert_eq!(missing.map_expect("present"), None);
    }

    #[test]
    fn unwraps_an_option_inside_a_result() {
        let value: Result<Option<u8>, String> = Ok(Some(3));
        assert_eq!(value.map_expect("present").expect("ok"), 3);

        let failed: Result<Option<u8>, String> = Err("nope".to_owned());
        assert_eq!(failed.map_expect("present").expect_err("err"), "nope");
    }

    #[test]
    #[should_panic(expected = "the message")]
    fn an_err_panics_with_the_message() {
        let value: Option<Result<u8, String>> = Some(Err("boom".to_owned()));

        let _ = value.map_expect("the message");
    }
}
