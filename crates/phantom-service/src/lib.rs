#![recursion_limit = "192"]

pub mod account_data;
pub mod admin;
pub mod appservice;
pub mod client;
pub mod config;
pub mod deactivate;
pub mod emergency;
pub mod federation;
pub mod fetcher;
pub mod key_backups;
pub mod media;
pub mod moderation;
pub mod oauth;
pub mod presence;
pub mod pusher;
pub mod resolver;
pub mod rooms;
pub mod runtime;
pub mod sending;
pub mod sendmail;
pub mod server_keys;
pub mod server_state;
pub mod sync;
pub mod tasks;
pub mod transaction_id;
pub mod uiaa;
pub mod updates;
pub mod users;

pub use self::runtime::{Args, Dep, Map, Service, Services, add, get, make_name, try_get};
