pub mod arrayvec;
pub mod bool;
pub mod bytes;
pub mod content_disposition;
pub mod diagnostics;
pub mod future;
pub mod hash;
pub mod json;
pub mod macros;
pub mod math;
pub mod matrix;
pub mod rand;
pub mod result;
pub mod runtime;
pub mod secret;
pub mod set;
pub mod stream;
pub mod sync;
pub mod text;
pub mod time;
pub mod url;

pub use self::{diagnostics::error::Error, result::Result, runtime::config::Config};

extern crate self as phantom_core;

diagnostics::info::rustc_flags_capture! {}

pub use ::{http, ruma, tracing};

pub use phantom_macros::implement;

pub use phantom_macros::recursion_depth;

pub use ctor::ctor;

#[inline]
pub fn exchange<T>(state: &mut T, source: T) -> T {
    std::mem::replace(state, source)
}
