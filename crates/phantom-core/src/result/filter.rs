use super::Result;

/// [`Option::filter`] for [`Result`], with the predicate reporting *why* it
/// rejected the value rather than discarding that along with it.
pub trait Filter<T, E> {
    /// Passes an `Ok` value through the predicate, turning a rejection into the
    /// error the predicate returned.
    #[must_use]
    fn filter<P, U>(self, predicate: P) -> Self
    where
        P: FnOnce(&T) -> Result<(), U>,
        E: From<U>;
}

impl<T, E> Filter<T, E> for Result<T, E> {
    #[inline]
    fn filter<P, U>(self, predicate: P) -> Self
    where
        P: FnOnce(&T) -> Result<(), U>,
        E: From<U>,
    {
        self.and_then(move |t| predicate(&t).map(move |()| t).map_err(Into::into))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn even(value: &u8) -> Result<(), String> {
        value
            .is_multiple_of(2)
            .then_some(())
            .ok_or_else(|| format!("{value} is odd"))
    }

    #[test]
    fn an_accepted_value_passes_through() {
        let result: Result<u8, String> = Ok(4);

        assert_eq!(result.filter(even).expect("accepted"), 4);
    }

    #[test]
    fn a_rejected_value_becomes_the_predicates_error() {
        let result: Result<u8, String> = Ok(5);

        assert_eq!(result.filter(even).expect_err("rejected"), "5 is odd");
    }

    #[test]
    fn an_existing_error_is_left_alone() {
        let result: Result<u8, String> = Err("already failed".to_owned());

        assert_eq!(
            result.filter(even).expect_err("unchanged"),
            "already failed"
        );
    }
}
