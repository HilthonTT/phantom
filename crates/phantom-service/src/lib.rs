#![recursion_limit = "192"]

pub mod accounts;
pub mod auth;
pub mod media;
pub mod net;
pub mod ops;
mod ratelimit;
pub mod rooms;
pub mod runtime;

pub use self::runtime::{Args, Dep, Map, Service, Services, add, get, make_name, try_get};
