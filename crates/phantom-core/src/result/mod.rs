//! The crate's `Result` alias and extension traits for it.

mod debug_inspect;
mod flat_ok;
mod log_debug_err;
mod result;

pub use self::{
    debug_inspect::DebugInspect, flat_ok::FlatOk, log_debug_err::LogDebugErr, result::Result,
};
