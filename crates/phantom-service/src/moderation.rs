//! Which remote servers this server refuses to deal with, and how far the
//! refusal goes.
//!
//! An operator's objection to a server is rarely all-or-nothing. Some servers
//! are not to be spoken to at all; some are fine to federate with but should
//! not have their room directory republished here; some are fine to talk to
//! but should not have their media pulled onto this server's disk. Those are
//! three config lists, and the mistake they invite is a call site that checks
//! one of them and forgets that the blanket list covers it too.
//!
//! So there is one question here — [`forbids`] — and the [`Restriction`] says
//! which of the three is being asked about. The blanket list is folded in by
//! this service rather than by each caller, which is the whole reason it
//! exists as a service rather than as three reads of [`Config`].
//!
//! Nothing is forbidden to this server itself. A server name that resolves to
//! us is answered locally and never crosses a network, so a pattern that
//! happens to match our own name is far more likely to be a stray `.` in a
//! regular expression than an operator asking to partition themselves.
//!
//! [`forbids`]: Service::forbids
//! [`Config`]: phantom_core::Config

use std::{fmt::Display, sync::Arc};

use phantom_core::{Result, implement, server::Server};
use regex::RegexSet;
use ruma::ServerName;

use crate::{Dep, server_state};

pub struct Service {
    services: Services,
}

struct Services {
    server: Arc<Server>,
    server_state: Dep<server_state::Service>,
}

/// What a remote server is being refused.
///
/// Ordered from widest to narrowest, which is also the order the config lists
/// subsume each other in: anything [`Federation`] forbids is forbidden for the
/// other two as well.
///
/// [`Federation`]: Restriction::Federation
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Restriction {
    /// Exchanging federation traffic at all.
    Federation,

    /// Querying its published room directory, and publishing its rooms in
    /// ours.
    RoomDirectory,

    /// Downloading the media it hosts.
    Media,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                server_state: args.depend::<server_state::Service>("server_state"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Whether `server` is refused the thing `restriction` names.
///
/// The blanket `forbidden_remote_server_names` is consulted for every
/// restriction, so a caller asking about media does not also have to ask
/// whether the server is federated with at all.
#[implement(Service)]
#[must_use]
pub fn forbids(&self, server: &ServerName, restriction: Restriction) -> bool {
    if self.services.server_state.server_is_ours(server) {
        return false;
    }

    let config = &self.services.server.config;
    let host = server.host();

    config.forbidden_remote_server_names.is_match(host)
        || match restriction {
            Restriction::Federation => false,
            Restriction::RoomDirectory => config
                .forbidden_remote_room_directory_server_names
                .is_match(host),
            Restriction::Media => config.forbidden_remote_media_server_names.is_match(host),
        }
}

/// The patterns that made [`forbids`] answer true, for an error message.
///
/// A refusal that only says "forbidden" leaves an operator grepping their
/// config for which of three lists did it. Building the list costs a second
/// match against the same patterns, which is why it is separate from the
/// predicate rather than returned by it.
///
/// [`forbids`]: Service::forbids
#[implement(Service)]
#[must_use]
pub fn why_forbidden(&self, server: &ServerName, restriction: Restriction) -> Vec<String> {
    let config = &self.services.server.config;
    let host = server.host();

    let narrow = match restriction {
        Restriction::Federation => None,
        Restriction::RoomDirectory => Some(&config.forbidden_remote_room_directory_server_names),
        Restriction::Media => Some(&config.forbidden_remote_media_server_names),
    };

    matched(&config.forbidden_remote_server_names, host)
        .chain(narrow.into_iter().flat_map(|set| matched(set, host)))
        .collect()
}

/// The patterns in `set` that match `host`.
fn matched<'a>(set: &'a RegexSet, host: &'a str) -> impl Iterator<Item = String> + 'a {
    set.matches(host)
        .into_iter()
        .filter_map(|index| set.patterns().get(index))
        .cloned()
}

impl Display for Restriction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Federation => "federation",
            Self::RoomDirectory => "room directory access",
            Self::Media => "media downloads",
        };

        f.write_str(name)
    }
}
