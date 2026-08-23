use super::Result;
use crate::Error;

/// Whether a [`Result`] failed because something was not found.
///
/// The check is often used to turn an `Err` back into the `Ok(None)` a caller
/// wanted, where a lookup that misses is not an error.
pub trait NotFound<T> {
    #[must_use]
    fn is_not_found(&self) -> bool;
}

impl<T> NotFound<T> for Result<T, Error> {
    #[inline]
    fn is_not_found(&self) -> bool {
        self.as_ref().is_err_and(Error::is_not_found)
    }
}

#[cfg(test)]
mod tests {
    use ruma::api::error::ErrorKind;

    use super::*;

    #[test]
    fn only_not_found_errors_report_true() {
        let missing: Result<()> = Err(Error::Request(
            ErrorKind::NotFound,
            "no such room".into(),
            http::StatusCode::NOT_FOUND,
        ));
        assert!(missing.is_not_found());

        let other: Result<()> = Err(crate::err!(Database("unreadable")));
        assert!(!other.is_not_found());

        let ok: Result<()> = Ok(());
        assert!(!ok.is_not_found());
    }
}
