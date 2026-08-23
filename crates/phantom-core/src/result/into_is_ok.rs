use super::Result;

/// [`Result::is_ok`] by value, for combinator position.
///
/// `Result::is_ok` borrows, so it cannot be named as the function in
/// `.map(..)`/`.then(..)` over owned results without a closure.
pub trait IntoIsOk<T, E> {
    fn into_is_ok(self) -> bool;
}

impl<T, E> IntoIsOk<T, E> for Result<T, E> {
    #[inline]
    fn into_is_ok(self) -> bool {
        self.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_results_to_whether_they_succeeded() {
        let results: Vec<Result<u8, ()>> = vec![Ok(1), Err(()), Ok(2)];

        let flags: Vec<_> = results.into_iter().map(IntoIsOk::into_is_ok).collect();

        assert_eq!(flags, [true, false, true]);
    }
}
