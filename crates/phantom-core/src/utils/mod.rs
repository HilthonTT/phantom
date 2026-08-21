//! Assorted helpers shared across the crate.

pub mod config;
pub mod future;
pub mod hash;
pub mod json;
pub mod math;
pub mod rand;
pub mod result;
pub mod stream;

pub use self::json::{deserialize_from_str, to_canonical_object};
pub use stream::IterStream;
