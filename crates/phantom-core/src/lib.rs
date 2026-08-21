//! Shared types and logic for the phantom homeserver.

pub mod alloc;
pub mod debug;
mod macros;
pub mod utils;

/// Re-exported so modules can spell the attribute as `#[crate::implement]`,
/// and `check.rs`-style files can `use crate::implement`.
pub use phantom_macros::implement;

/// Re-exported so allocator modules can spell the pre-main initializer as
/// `#[crate::ctor]`.
#[cfg(feature = "jemalloc")]
pub use ctor::ctor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("{0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("{0}")]
    TryFromInt(#[from] std::num::TryFromIntError),

    #[error("arithmetic operation overflowed")]
    Overflow,

    #[error("capacity exceeded: {0}")]
    Capacity(String),

    #[error("lock poisoned: {0}")]
    Poison(String),
}

// `PoisonError` borrows the guard it wraps, so it cannot be captured by value in
// the enum; flatten it to its message instead.
impl<T> From<std::sync::PoisonError<T>> for Error {
    fn from(error: std::sync::PoisonError<T>) -> Self {
        Self::Poison(error.to_string())
    }
}

// `CapacityError` borrows the element that did not fit, same as above.
#[cfg(feature = "jemalloc")]
impl<T> From<arrayvec::CapacityError<T>> for Error {
    fn from(error: arrayvec::CapacityError<T>) -> Self {
        Self::Capacity(error.to_string())
    }
}

pub type Result<T = (), E = Error> = std::result::Result<T, E>;
