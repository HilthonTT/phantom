//! Everything the server knows about rooms.
//!
//! One area of room state per submodule, each its own service registered
//! under its own name. [`Service`] here is not one of them: it owns no state
//! and has no worker, it is only the handle that groups them, so that a
//! caller reaches the room directory as `services.rooms.directory` rather
//! than having `rooms_directory` sit at the top of [`crate::Services`]
//! alongside two dozen siblings.
//!
//! Because it is not a service, it is not in the registry either — the
//! submodules are, individually, and a [`Dep`](crate::Dep) is taken on one of
//! those rather than on the group.

pub mod auth_chain;
pub mod directory;
pub mod lazy_loading;
pub mod metadata;
pub mod outlier;
pub mod short;
pub mod timeline;
pub mod user;

use std::sync::Arc;

/// The room services, grouped.
pub struct Service {
    pub directory: Arc<directory::Service>,
    pub short: Arc<short::Service>,
    pub timeline: Arc<timeline::Service>,
}
