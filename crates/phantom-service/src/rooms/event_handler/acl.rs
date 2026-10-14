//! Server ACLs (`m.room.server_acl`): which servers a room refuses to hear
//! from.
//!
//! Every inbound federation endpoint scoped to a room checks the requesting
//! server here before acting: `/send`'s PDUs (by origin and by sender) and its
//! typing and receipt EDUs, the make/send join, leave and knock handshakes,
//! `/invite`, `/backfill`, `/get_missing_events`, `/state`, `/state_ids`,
//! `/event_auth`, `/event` and `/timestamp_to_event`.

use phantom_core::{Err, Result, debug, implement, trace};
use ruma::{
    RoomId, ServerName,
    events::{StateEventType, room::server_acl::RoomServerAclEventContent},
};

use super::Service;

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub async fn acl_check(&self, origin: &ServerName, room_id: &RoomId) -> Result {
    if self.services.server_state.server_is_ours(origin) {
        return Ok(());
    }

    let Ok(acl) = self
        .services
        .state_accessor
        .room_state_get_content::<RoomServerAclEventContent>(
            room_id,
            &StateEventType::RoomServerAcl,
            "",
        )
        .await
    else {
        trace!("No ACL in {room_id}");
        return Ok(());
    };

    if acl_allows(&acl, origin) {
        trace!("server {origin} is allowed by the ACL in {room_id}");
        return Ok(());
    }

    debug!("Server {origin} was denied by the ACL in {room_id}");

    Err!(Request(Forbidden("Server was denied by the room's ACL.")))
}

/// Whether the ACL lets the server in.
///
/// An ACL whose allow list is empty denies every server by the letter of the
/// spec, this one included, which leaves the room unusable for everyone. It
/// is ignored instead, as conduwuit does: such an event is far likelier to be
/// a mistake than a wish to end the room.
#[must_use]
pub fn acl_allows(acl: &RoomServerAclEventContent, server: &ServerName) -> bool {
    acl.allow.is_empty() || acl.is_allowed(server)
}

#[cfg(test)]
mod tests {
    use ruma::{events::room::server_acl::RoomServerAclEventContent, server_name};

    use super::acl_allows;

    fn acl(allow_ip_literals: bool, allow: &[&str], deny: &[&str]) -> RoomServerAclEventContent {
        RoomServerAclEventContent::new(
            allow_ip_literals,
            allow.iter().map(|&s| s.to_owned()).collect(),
            deny.iter().map(|&s| s.to_owned()).collect(),
        )
    }

    #[test]
    fn allow_globs_match_hosts() {
        let acl = acl(false, &["*.example.org", "matrix.org"], &[]);

        assert!(acl_allows(&acl, server_name!("a.example.org")));
        assert!(acl_allows(&acl, server_name!("matrix.org")));
        assert!(!acl_allows(&acl, server_name!("example.org")));
        assert!(!acl_allows(&acl, server_name!("evil.com")));
    }

    #[test]
    fn deny_wins_over_allow() {
        let acl = acl(false, &["*"], &["evil.com", "*.evil.com"]);

        assert!(acl_allows(&acl, server_name!("good.com")));
        assert!(!acl_allows(&acl, server_name!("evil.com")));
        assert!(!acl_allows(&acl, server_name!("sub.evil.com")));
    }

    #[test]
    fn the_port_is_ignored() {
        let acl = acl(false, &["*"], &["evil.com"]);

        assert!(!acl_allows(&acl, server_name!("evil.com:8448")));
    }

    #[test]
    fn single_character_wildcards() {
        let acl = acl(false, &["*"], &["ev?l.com"]);

        assert!(!acl_allows(&acl, server_name!("evil.com")));
        assert!(!acl_allows(&acl, server_name!("eval.com")));
        assert!(acl_allows(&acl, server_name!("eviil.com")));
    }

    #[test]
    fn ip_literals_follow_the_flag() {
        let refuses = acl(false, &["*"], &[]);
        let allows = acl(true, &["*"], &[]);

        assert!(!acl_allows(&refuses, server_name!("1.2.3.4")));
        assert!(!acl_allows(&refuses, server_name!("[::1]:8448")));
        assert!(acl_allows(&allows, server_name!("1.2.3.4")));
    }

    #[test]
    fn an_empty_allow_list_is_ignored() {
        let acl = acl(false, &[], &["evil.com"]);

        assert!(acl_allows(&acl, server_name!("evil.com")));
        assert!(acl_allows(&acl, server_name!("good.com")));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let acl = acl(false, &["*"], &["Evil.COM"]);

        assert!(!acl_allows(&acl, server_name!("evil.com")));
    }
}
