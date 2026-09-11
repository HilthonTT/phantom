use phantom_core::{Err, Result, debug, implement, trace};
use ruma::{RoomId, ServerName, events::StateEventType};

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
