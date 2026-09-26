//! Running the server itself: configuration, identity, storage, administration, background tasks and the operator's policy.

pub mod admin;
pub mod appservice;
pub mod config;
pub mod emergency;
pub(crate) mod migrations;
pub mod moderation;
pub mod server_state;
pub mod storage;
pub mod tasks;
pub mod updates;
