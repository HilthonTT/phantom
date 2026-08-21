//! The crate's `Result` alias and extension traits for it.

mod debug_inspect;
mod result;

pub use self::{debug_inspect::DebugInspect, result::Result};
