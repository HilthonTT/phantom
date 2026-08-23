//! Assorted helpers with no home of their own.
//!
//! Anything here should be a general-purpose utility. Concepts the server is
//! actually built out of — `config`, `error`, `result` — live at the crate
//! root instead.

pub mod arrayvec;
pub mod bool;
pub mod bytes;
pub mod content_disposition;
pub mod debug;
pub mod defer;
pub mod future;
pub mod hash;
pub mod html;
pub mod json;
pub mod math;
pub mod mutex_map;
pub mod rand;
pub mod set;
pub mod stream;
pub mod time;

pub mod sys;

pub use self::{
    arrayvec::ArrayVecExt,
    bool::BoolExt,
    debug::slice_truncated as debug_slice_truncated,
    future::TryExtExt as TryFutureExtExt,
    html::Escape as HtmlEscape,
    json::{deserialize_from_str, to_canonical_object},
    mutex_map::{Guard as MutexMapGuard, MutexMap},
    rand::{shuffle, string as random_string},
    stream::{IterStream, ReadyExt, Tools as StreamTools, TryReadyExt},
    sys::compute::available_parallelism,
    time::{
        now_millis as millis_since_unix_epoch, rfc2822_from_seconds, timepoint_ago,
        timepoint_from_now,
    },
};

/// Replaces `state` with `source`, returning the previous value.
///
/// Exists as a free function so [`crate::scope_restore!`] can spell it as
/// `$crate::utils::exchange` without callers importing `std::mem`.
#[inline]
pub fn exchange<T>(state: &mut T, source: T) -> T {
    std::mem::replace(state, source)
}
