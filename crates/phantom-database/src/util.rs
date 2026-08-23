//! Adapting the database engine's error type to phantom's.

use std::io;

use phantom_core::{Error, Result};
use rocksdb::ErrorKind;

/// Lifts an engine result into a phantom one.
#[inline]
pub(crate) fn result<T>(res: Result<T, rocksdb::Error>) -> Result<T> {
    res.map_err(map_err)
}

/// [`result`] in combinator position, for `.or_else(or_else)` on the engine
/// results that are more naturally handled where they are produced.
#[inline]
pub(crate) fn or_else<T>(e: rocksdb::Error) -> Result<T> {
    Err(map_err(e))
}

/// Translates an engine error into [`Error::Io`].
///
/// The engine reports failures as a string plus a coarse kind. Mapping that
/// kind onto [`io::ErrorKind`] keeps the distinction between, say, a busy
/// column and a corrupt one legible to callers that would otherwise only see
/// prose.
pub(crate) fn map_err(e: rocksdb::Error) -> Error {
    let kind = io_error_kind(&e.kind());
    let string = e.into_string();

    io::Error::new(kind, string).into()
}

fn io_error_kind(e: &ErrorKind) -> io::ErrorKind {
    match e {
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::Corruption => io::ErrorKind::InvalidData,
        ErrorKind::InvalidArgument => io::ErrorKind::InvalidInput,
        ErrorKind::Aborted => io::ErrorKind::Interrupted,
        ErrorKind::NotSupported => io::ErrorKind::Unsupported,
        ErrorKind::CompactionTooLarge => io::ErrorKind::FileTooLarge,
        ErrorKind::MergeInProgress | ErrorKind::Busy => io::ErrorKind::ResourceBusy,
        ErrorKind::Expired | ErrorKind::TimedOut => io::ErrorKind::TimedOut,
        ErrorKind::Incomplete | ErrorKind::TryAgain => io::ErrorKind::WouldBlock,
        ErrorKind::ColumnFamilyDropped
        | ErrorKind::ShutdownInProgress
        | ErrorKind::IOError
        | ErrorKind::Unknown => io::ErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_error_kinds_map_onto_io_kinds() {
        assert_eq!(io_error_kind(&ErrorKind::NotFound), io::ErrorKind::NotFound);
        assert_eq!(
            io_error_kind(&ErrorKind::Corruption),
            io::ErrorKind::InvalidData,
            "corruption is bad data, not a missing file"
        );
        assert_eq!(io_error_kind(&ErrorKind::Busy), io::ErrorKind::ResourceBusy);
        assert_eq!(
            io_error_kind(&ErrorKind::TryAgain),
            io::ErrorKind::WouldBlock,
            "a retryable read must not look like a hard failure"
        );
        assert_eq!(io_error_kind(&ErrorKind::Unknown), io::ErrorKind::Other);
    }
}
