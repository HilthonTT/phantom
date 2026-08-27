//! The crate's `Result` alias and extension traits for it.

mod alias;
mod debug_inspect;
mod filter;
mod flat_ok;
mod into_is_ok;
mod log_debug_err;
mod log_err;
mod map_expect;
mod not_found;
mod unwrap_infallible;
mod unwrap_or_err;

pub use self::{
    alias::Result, debug_inspect::DebugInspect, filter::Filter, flat_ok::FlatOk,
    into_is_ok::IntoIsOk, log_debug_err::LogDebugErr, log_err::LogErr, map_expect::MapExpect,
    not_found::NotFound, unwrap_infallible::UnwrapInfallible, unwrap_or_err::UnwrapOrErr,
};
