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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Restriction {
    Federation,

    RoomDirectory,

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

#[implement(Service)]
#[must_use]
pub fn forbids(&self, server: &ServerName, restriction: Restriction) -> bool {
    if self.services.server_state.server_is_ours(server) {
        return false;
    }

    let config = &self.services.server.config;
    let host = server.host();

    config
        .federation
        .forbidden_remote_server_names
        .is_match(host)
        || match restriction {
            Restriction::Federation => false,
            Restriction::RoomDirectory => config
                .federation
                .forbidden_remote_room_directory_server_names
                .is_match(host),
            Restriction::Media => config
                .federation
                .forbidden_remote_media_server_names
                .is_match(host),
        }
}

#[implement(Service)]
#[must_use]
pub fn why_forbidden(&self, server: &ServerName, restriction: Restriction) -> Vec<String> {
    let config = &self.services.server.config;
    let host = server.host();

    let narrow = match restriction {
        Restriction::Federation => None,
        Restriction::RoomDirectory => Some(
            &config
                .federation
                .forbidden_remote_room_directory_server_names,
        ),
        Restriction::Media => Some(&config.federation.forbidden_remote_media_server_names),
    };

    matched(&config.federation.forbidden_remote_server_names, host)
        .chain(narrow.into_iter().flat_map(|set| matched(set, host)))
        .collect()
}

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
