#![recursion_limit = "192"]

pub mod accounts;
// Unfinished admin command processor: it needs clap and a `console` feature
// before it builds. Remove this cfg when picking it back up.
#[cfg(disable)]
pub mod admin;
pub mod auth;
pub mod media;
pub mod net;
pub mod ops;
mod ratelimit;
pub mod rooms;
pub mod runtime;

pub use self::runtime::{Args, Dep, Map, Service, Services, add, get, make_name, try_get};
