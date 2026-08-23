use std::fmt::Display;

use tracing::Level;

use super::Result;
use crate::error;

/// Logs the error in an `Err` and passes the result through, for the common
/// case of a failure worth reporting but not worth propagating.
pub trait LogErr<T, E: Display> {
    /// Logs an `Err` at `level`.
    #[must_use]
    fn err_log(self, level: Level) -> Self;

    /// Logs an `Err` at [`Level::ERROR`].
    #[must_use]
    fn log_err(self) -> Self
    where
        Self: Sized,
    {
        self.err_log(Level::ERROR)
    }
}

impl<T, E: Display> LogErr<T, E> for Result<T, E> {
    #[inline]
    fn err_log(self, level: Level) -> Self {
        self.inspect_err(|error| error::inspect_log_level(&error, level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_passes_through_unchanged() {
        let ok: Result<u8, String> = Ok(7);
        assert_eq!(ok.log_err().expect("unchanged"), 7);

        let err: Result<u8, String> = Err("bad".to_owned());
        assert_eq!(err.err_log(Level::WARN).expect_err("unchanged"), "bad");
    }
}
