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
//!
//! [`runtime`] holds all of that machinery; every other module here is one
//! service.

pub mod account_data;
pub mod appservice;
pub mod client;
pub mod config;
pub mod emergency;
pub mod federation;
pub mod key_backups;
pub mod presence;
pub mod resolver;
pub mod rooms;
pub mod runtime;
pub mod sending;
pub mod server_keys;
pub mod server_state;
pub mod transaction_id;
pub mod uiaa;
pub mod users;

pub use self::runtime::{Args, Dep, Map, Service, Services, add, get, make_name, try_get};
