//! Shared types and logic for the phantom homeserver.
//!
//! Everything in here is depended on by every crate above it, so the module
//! layout is deliberately flat: a module is named for its subject, and a
//! module full of extension traits is named for the type it extends. There is
//! no catch-all — a helper that fits nowhere is a sign the subject it belongs
//! to has not been named yet.
//!
//! # What lives where
//!
//! **The server itself.** [`server`] is the handle a running instance is
//! driven through, [`config`] is what an operator set it up with, [`alloc`]
//! picks the allocator it runs on, and [`metrics`] and [`sys`] report on the
//! process and the host it is running on.
//!
//! **Diagnostics.** [`error`] is the crate's error type and [`result`] its
//! `Result` alias plus the combinators for it. [`log`] is the tracing
//! subscriber and the macros that feed it; [`debugger`] is the breakpoint trap
//! for running under `gdb`; [`info`] is who this build says it is.
//!
//! **The protocol.** [`matrix`] holds the event types and state resolution.
//!
//! **Language-level support**, each named for what it extends or produces:
//! [`arrayvec`], [`bool`], [`bytes`], [`future`], [`hash`], [`json`],
//! [`macros`], [`math`], [`rand`], [`set`], [`stream`], [`sync`], [`text`] and
//! [`time`].

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

// So that a macro expanding to `phantom_core::…` — which is what it must emit
// for the downstream crates that are its usual callers — also resolves when it
// is invoked here.
extern crate self as phantom_core;

// Records this crate's compiler flags for `info::rustc`, which is how a build
// reports the cargo features it was actually compiled with.
info::rustc_flags_capture! {}

pub use ::{http, ruma, tracing};

/// Re-exported so modules can spell the attribute as `#[crate::implement]`.
pub use phantom_macros::implement;

/// Re-exported so a struct can spell the attribute as
/// `#[crate::recursion_depth]`.
pub use phantom_macros::recursion_depth;

/// Re-exported so allocator modules and `info::rustc`'s flag registration can
/// spell the pre-main initializer as `#[crate::ctor]`.
pub use ctor::ctor;

/// Replaces `state` with `source`, returning the previous value.
///
/// Exists as a free function at the crate root so [`scope_restore!`] can spell
/// it as `$crate::exchange` without callers importing `std::mem`.
#[inline]
pub fn exchange<T>(state: &mut T, source: T) -> T {
    std::mem::replace(state, source)
}
