//! Talking to other homeservers.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Federation {
    /// Serve this server's public room directory to other servers over
    /// federation.
    ///
    /// Leaving this off keeps the directory from being crawled by remote
    /// spiders, at the cost of your rooms not appearing in other servers'
    /// directory searches.
    #[serde(default)]
    pub allow_public_room_directory_over_federation: bool,

    /// Send device display names to other servers, so remote users see what a
    /// local user named their session.
    ///
    /// Off by default: the names are frequently identifying, and nothing in
    /// the protocol needs them.
    #[serde(default)]
    pub allow_device_name_federation: bool,

    /// Notary servers to gather other servers' public keys from, when this
    /// server does not already hold a key it needs.
    ///
    /// example: ["matrix.org", "tchncs.de"]
    ///
    /// default: ["matrix.org"]
    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,

    /// Ask the notaries in `trusted_servers` for a key before asking the
    /// server the key belongs to.
    ///
    /// Asking the origin first is the safer order: a notary that has been
    /// compromised can only answer for keys it was asked about, and it is
    /// only asked once the origin has failed to answer. Asking the notaries
    /// first is faster, since one notary can answer for many servers at once.
    #[serde(default)]
    pub query_trusted_key_servers_first: bool,

    /// Ask the notaries first, but only while joining a room.
    ///
    /// A join gathers keys from every server in the room, which is where the
    /// per-origin round trips are most noticeable; this bounds the exposure
    /// to a compromised notary to that one operation. Ignored where
    /// `query_trusted_key_servers_first` is already on.
    #[serde(default = "true_fn")]
    pub query_trusted_key_servers_first_on_join: bool,

    /// Only ever ask the notaries in `trusted_servers` for keys, and never
    /// the server a key belongs to.
    ///
    /// For a cluster behind a notary it operates itself. With no reachable
    /// notary holding a key, that key is simply never acquired.
    #[serde(default)]
    pub only_query_trusted_key_servers: bool,

    /// Servers to ask a notary about in one batched request.
    ///
    /// default: 256
    #[serde(default = "default_trusted_server_batch_size")]
    pub trusted_server_batch_size: usize,

    /// Send federation requests to other servers.
    ///
    /// With this off the server still answers what arrives, but never
    /// initiates a request of its own, which includes fetching the signing
    /// keys needed to verify a remote event.
    #[serde(default = "true_fn")]
    pub allow_federation: bool,

    /// Servers this server refuses to send federation requests to, as regular
    /// expressions matched against the server name.
    ///
    /// A plain word is a valid pattern, and matches anywhere in the name.
    ///
    /// example: ["badserver\\.tld$", "badphrase", "19dollarfortnitecards"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_server_names: RegexSet,

    /// How long the server will spend fetching and placing the events before
    /// an event that arrived with a gap in front of it, in seconds.
    ///
    /// A server that has been unreachable for a while hands back an event
    /// whose history this server is missing entirely, and closing that gap
    /// event by event can take longer than the outage did. When the budget
    /// runs out the event is still accepted — its state comes from the sending
    /// server rather than from this server's own record — and the rest of the
    /// gap is left to backfill.
    ///
    /// default: 300
    #[serde(default = "default_federation_prev_event_budget_s")]
    pub federation_prev_event_budget_s: u64,

    /// Servers whose public room directory this server will neither query nor
    /// republish, as regular expressions matched against the server name.
    ///
    /// Narrower than `forbidden_remote_server_names`, which already covers
    /// the directory along with everything else — this is for a server worth
    /// federating with whose room directory is not worth showing.
    ///
    /// example: ["nsfwserver\\.tld$"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_room_directory_server_names: RegexSet,

    /// Servers this server will not download media from, as regular
    /// expressions matched against the server name.
    ///
    /// Narrower than `forbidden_remote_server_names` in the same way: the
    /// server is federated with, but nothing it hosts is fetched onto this
    /// server's disk. Media already downloaded is not removed — the admin
    /// command that purges it is.
    ///
    /// example: ["badserver\\.tld$"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_media_server_names: RegexSet,

    /// Send federation requests to this server itself, which nothing but a
    /// bug or a development setup has a reason to do.
    #[serde(default)]
    pub federation_loopback: bool,
}
