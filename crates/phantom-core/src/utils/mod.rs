//! Assorted helpers with no home of their own.
//!
//! Anything here should be a general-purpose utility. Concepts the server is
//! actually built out of — `config`, `error`, `result` — live at the crate
//! root instead.

pub mod arrayvec;
pub mod bool;
pub mod bytes;
pub mod future;
pub mod hash;
pub mod json;
pub mod math;
pub mod rand;
pub mod stream;

pub use self::{
    json::{deserialize_from_str, to_canonical_object},
    stream::IterStream,
};
