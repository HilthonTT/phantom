//! Everything that talks to another host: HTTP clients, server-name resolution, federation requests, signing keys, the outbound queue, identity servers, SMTP and SMS.

pub mod client;
pub mod federation;
pub mod fetcher;
pub mod identity;
pub mod resolver;
pub mod sending;
pub mod sendmail;
pub mod server_keys;
pub mod sms;
