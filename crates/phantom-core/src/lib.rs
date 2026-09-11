pub mod alloc;
pub mod arrayvec;
pub mod bool;
pub mod bytes;
pub mod config;
pub mod content_disposition;
pub mod debugger;
pub mod error;
pub mod future;
pub mod hash;
pub mod info;
pub mod json;
pub mod log;
pub mod macros;
pub mod math;
pub mod matrix;
pub mod metrics;
pub mod rand;
pub mod result;
pub mod secret;
pub mod server;
pub mod set;
pub mod stream;
pub mod sync;
pub mod sys;
pub mod text;
pub mod time;
pub mod url;

pub use self::{config::Config, error::Error, result::Result};

extern crate self as phantom_core;

info::rustc_flags_capture! {}

pub use ::{http, ruma, tracing};

pub use phantom_macros::implement;

pub use phantom_macros::recursion_depth;

pub use ctor::ctor;

#[inline]
pub fn exchange<T>(state: &mut T, source: T) -> T {
    std::mem::replace(state, source)
}
