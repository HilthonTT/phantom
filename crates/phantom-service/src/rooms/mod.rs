//! Everything the server knows about rooms.
//!
//! One area of room state per submodule, each its own service registered
//! under its own name. [`Services`] here is not one of them: it owns no state
//! and has no worker, it is only the handle that groups them, so that a
//! caller reaches the room directory as `services.rooms.directory` rather
//! than having `rooms_directory` sit at the top of [`crate::Services`]
//! alongside two dozen siblings.
//!
//! Because it is not a service, it is not in the registry either — the
//! submodules are, individually, and a [`Dep`](crate::Dep) is taken on one of
//! those rather than on the group.

pub mod alias;
pub mod auth_chain;
pub mod directory;
pub mod lazy_loading;
pub mod metadata;
pub mod outlier;
pub mod pdu_metadata;
pub mod read_receipt;
pub mod search;
pub mod short;
pub mod spaces;
pub mod state;
pub mod state_accessor;
pub mod state_cache;
pub mod state_compressor;
pub mod threads;
pub mod timeline;
pub mod typing;
pub mod user;

use std::sync::Arc;

/// The room services, grouped.
pub struct Services {
    pub alias: Arc<alias::Service>,
    pub directory: Arc<directory::Service>,
    pub short: Arc<short::Service>,
    pub spaces: Arc<spaces::Service>,
    pub state: Arc<state::Service>,
    pub state_accessor: Arc<state_accessor::Service>,
    pub state_cache: Arc<state_cache::Service>,
    pub state_compressor: Arc<state_compressor::Service>,
    pub search: Arc<search::Service>,
    pub read_receipt: Arc<read_receipt::Service>,
    pub timeline: Arc<timeline::Service>,
    pub outlier: Arc<outlier::Service>,
    pub metadata: Arc<metadata::Service>,
    pub auth_chain: Arc<auth_chain::Service>,
    pub pdu_metadata: Arc<pdu_metadata::Service>,
    pub lazy_loading: Arc<lazy_loading::Service>,
    pub threads: Arc<threads::Service>,
    pub typing: Arc<typing::Service>,
    pub user: Arc<user::Service>,
}
