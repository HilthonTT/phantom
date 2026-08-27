//! The crate's `Result` alias.

pub type Result<T = (), E = crate::Error> = std::result::Result<T, E>;
