//! Shared types and logic for the phantom homeserver.

pub mod alloc;
pub mod config;
pub mod debug;
pub mod error;
pub mod log;
pub mod macros;
pub mod result;
pub mod utils;

pub use self::{config::Config, error::Error, result::Result};

// Re-exported for the macros in `error::err`, which spell these as
// `$crate::http` / `$crate::ruma` / `$crate::tracing` so callers do not need
// the crates in scope themselves.
pub use ::{http, ruma, tracing};

/// Re-exported so modules can spell the attribute as `#[crate::implement]`.
pub use phantom_macros::implement;

/// Re-exported so allocator modules can spell the pre-main initializer as
/// `#[crate::ctor]`.
#[cfg(feature = "jemalloc")]
pub use ctor::ctor;
