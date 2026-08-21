//! Shared types and logic for the phantom homeserver.

pub mod alloc;
pub mod config;
pub mod debug;
pub mod error;
mod macros;
pub mod result;
pub mod utils;

pub use self::{
    config::Config,
    error::{Error, Result},
};

/// Re-exported so modules can spell the attribute as `#[crate::implement]`.
pub use phantom_macros::implement;

/// Re-exported so allocator modules can spell the pre-main initializer as
/// `#[crate::ctor]`.
#[cfg(feature = "jemalloc")]
pub use ctor::ctor;
