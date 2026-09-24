//! Choosing and ordering the servers a membership transition may ask.
//!
//! Both `join` and `knock` need this, so it sits beside them rather than inside
//! either one.

use std::collections::HashSet;

use futures::StreamExt;
use phantom_core::{debug_info, implement, rand::shuffle};
use ruma::{OwnedServerName, RoomId, RoomOrAliasId, UserId};

use super::{Service, sender_servers};

#[implement(Service)]
pub(super) async fn servers_for_room(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    orig_room_id: Option<&RoomOrAliasId>,
    via: &[OwnedServerName],
) -> Vec<OwnedServerName> {
    let state_cache = &self.services.state_cache;

    let mut additional_servers: Vec<OwnedServerName> = state_cache
        .servers_invite_via(room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    if let Ok(invite_state) = state_cache.invite_state(user_id, room_id).await {
        additional_servers.extend(sender_servers(&invite_state));
    }

    let mut servers = via.to_vec();
    shuffle(&mut servers);

    let has_remote_via = via
        .iter()
        .any(|server| !self.services.server_state.server_is_ours(server));

    if !has_remote_via {
        if let Some(server_name) = room_id.server_name() {
            servers.insert(0, server_name.to_owned());
        }

        if let Some(orig_server_name) = orig_room_id.and_then(RoomOrAliasId::server_name) {
            servers.insert(0, orig_server_name.to_owned());
        }
    }

    shuffle(&mut additional_servers);
    servers.extend(additional_servers);

    order_servers(
        servers,
        &self
            .services
            .server
            .config
            .membership
            .deprioritize_joins_through_servers,
    )
}

fn order_servers(
    servers: Vec<OwnedServerName>,
    deprioritized: &regex::RegexSet,
) -> Vec<OwnedServerName> {
    let mut seen = HashSet::new();

    let (mut preferred, demoted): (Vec<_>, Vec<_>) = servers
        .into_iter()
        .filter(|server| seen.insert(server.clone()))
        .partition(|server| !deprioritized.is_match(server.host()));

    preferred.extend(demoted);

    debug_info!(?preferred);

    preferred
}

#[cfg(test)]
mod tests {
    use regex::RegexSet;
    use ruma::OwnedServerName;

    use super::order_servers;

    fn names(servers: &[OwnedServerName]) -> Vec<&str> {
        servers.iter().map(AsRef::as_ref).collect()
    }

    #[test]
    fn duplicates_keep_their_first_position() {
        let servers = ["a.test", "b.test", "a.test", "c.test", "b.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let ordered = order_servers(servers.to_vec(), &RegexSet::empty());

        assert_eq!(names(&ordered), ["a.test", "b.test", "c.test"]);
    }

    #[test]
    fn deprioritized_servers_move_to_the_back_in_order() {
        let servers = ["matrix.org", "a.test", "sub.matrix.org", "b.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let deprioritized = RegexSet::new([r"matrix\.org"]).expect("valid");
        let ordered = order_servers(servers.to_vec(), &deprioritized);

        assert_eq!(
            names(&ordered),
            ["a.test", "b.test", "matrix.org", "sub.matrix.org"]
        );
    }

    #[test]
    fn adjacent_deprioritized_servers_are_all_moved() {
        let servers = ["matrix.org", "sub.matrix.org", "a.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let deprioritized = RegexSet::new([r"matrix\.org"]).expect("valid");
        let ordered = order_servers(servers.to_vec(), &deprioritized);

        assert_eq!(names(&ordered), ["a.test", "matrix.org", "sub.matrix.org"]);
    }
}
