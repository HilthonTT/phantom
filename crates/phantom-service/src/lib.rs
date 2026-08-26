//! The service layer of the phantom homeserver.
//!
//! A service is a long-lived singleton owning one area of the server's
//! behaviour. [`Services`] builds them all, holds them, and hands them to each
//! other; the manager behind it runs the worker loop each one may have and
//! restarts it if it panics.
//!
//! Services reach one another by name through a map of weak references rather
//! than by holding each other directly, which is what lets two of them depend
//! on each other without a reference cycle that never drops. See [`Dep`].

mod manager;
pub mod service;
pub mod services;

pub use self::{
    service::{Args, Dep, Map, Service, add, get, make_name, try_get},
    services::Services,
};
