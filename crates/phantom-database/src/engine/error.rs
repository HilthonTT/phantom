use std::io;

use phantom_core::{Error, Result};
use rocksdb::ErrorKind;

#[inline]
pub(crate) fn result<T>(res: Result<T, rocksdb::Error>) -> Result<T> {
    res.map_err(map_err)
}

#[inline]
pub(crate) fn or_else<T>(e: rocksdb::Error) -> Result<T> {
    Err(map_err(e))
}

#[inline]
pub(crate) fn is_incomplete(e: &rocksdb::Error) -> bool {
    e.kind() == ErrorKind::Incomplete
}

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
    fn incomplete_is_distinguished_from_other_kinds() {
        assert_eq!(
            io_error_kind(&ErrorKind::Incomplete),
            io::ErrorKind::WouldBlock,
            "an incomplete read is a retry, not a failure"
        );
    }

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
