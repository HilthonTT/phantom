//! Whether a server may take part in a room at all.
//!
//! `m.room.server_acl` is the room's own list of which servers are welcome. It
//! is state like any other, so a room that has never set one has no opinion
//! and every server is welcome; and because it is state, the answer can change
//! between one event and the next, which is why it is asked afresh rather than
//! cached.
//!
//! The check is deliberately narrow. It says nothing about whether *this*
//! server wants to talk to the sender — that is [`moderation`], asked
//! separately and for a different reason — and nothing about whether the event
//! is otherwise valid.
//!
//! [`moderation`]: crate::moderation

use phantom_core::{Err, Result, debug, implement, trace};
use ruma::{RoomId, ServerName, events::StateEventType};

use super::Service;

/// Refuses `origin` where the room's ACL does.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all)]
pub async fn acl_check(&self, origin: &ServerName, room_id: &RoomId) -> Result {
    // This server is not subject to a room's ACL. Its own events are how the
    // ACL got there, and a room that has locked us out is a room we are still
    // responsible for serving to our own users.
    if self.services.server_state.server_is_ours(origin) {
        return Ok(());
    }

    let Ok(acl) = self
        .services
        .state_accessor
        .room_state_get_content::<ruma::events::room::server_acl::RoomServerAclEventContent>(
            room_id,
            &StateEventType::RoomServerAcl,
            "",
        )
        .await
    else {
        trace!("No ACL in {room_id}");
        return Ok(());
    };

    // An ACL with an empty `allow` denies everyone, which is almost always a
    // mistake rather than an intention — the spec says as much — and a room
    // that has locked out every server including its own is a room nobody can
    // repair. Treating it as no ACL is what the spec recommends.
    if acl.allow.is_empty() {
        debug!("Ignoring broken ACL in {room_id}: the allow list is empty");
        return Ok(());
    }

    if acl.is_allowed(origin) {
        trace!("server {origin} is allowed by the ACL in {room_id}");
        return Ok(());
    }

    debug!("Server {origin} was denied by the ACL in {room_id}");

    Err!(Request(Forbidden("Server was denied by the room's ACL.")))
}
