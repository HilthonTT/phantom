//! Synchronization primitives phantom needs and `std` does not have.

pub mod mutex_map;

pub use self::mutex_map::{Guard as MutexMapGuard, MutexMap};
