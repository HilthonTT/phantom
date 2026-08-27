//! The machinery every service plugs into.
//!
//! [`contract`] is the trait a service implements and the arguments it is
//! built from, [`registry`] is the map they find each other through, and
//! [`manager`] runs each one's worker and restarts it if it panics.
//! [`services`] is the one place that names them all: it builds them in
//! order, holds the strong references, and starts and stops them together.

pub mod contract;
pub mod manager;
pub mod registry;
pub mod services;

pub use self::{
    contract::{Args, Service},
    manager::Manager,
    registry::{Dep, Map, add, get, make_name, try_get},
    services::Services,
};
