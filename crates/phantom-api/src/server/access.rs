use std::pin::pin;

use futures::{FutureExt, StreamExt, future::OptionFuture, join};
use phantom_core::{Err, Result, err};
use phantom_service::Services;
use ruma::{CanonicalJsonValue, EventId, RoomId, ServerName};

pub(super) struct AccessCheck<'a> {
    pub(super) services: &'a Services,
    pub(super) origin: &'a ServerName,
    pub(super) room_id: &'a RoomId,
    pub(super) event_id: Option<&'a EventId>,
}

impl AccessCheck<'_> {
    pub(super) async fn check(&self) -> Result {
        let rooms = &self.services.rooms;

        let acl_allows = rooms
            .event_handler
            .acl_check(self.origin, self.room_id)
            .map(|result| result.is_ok());

        let room_reachable = async {
            if rooms
                .state_cache
                .server_in_room(self.origin, self.room_id)
                .await
                || rooms.state_accessor.is_world_readable(self.room_id).await
            {
                return true;
            }

            let mut knocked = pin!(rooms.state_cache.room_members_knocked(self.room_id));
            knocked.next().await.is_some()
        };

        let server_can_see: OptionFuture<_> = self
            .event_id
            .map(|event_id| {
                rooms
                    .state_accessor
                    .server_can_see_event(self.origin, self.room_id, event_id)
            })
            .into();

        let (acl_allows, room_reachable, server_can_see) =
            join!(acl_allows, room_reachable, server_can_see);

        if !acl_allows {
            return Err!(Request(Forbidden("Server access denied.")));
        }

        if !room_reachable {
            return Err!(Request(Forbidden("Server is not in room.")));
        }

        if server_can_see == Some(false) {
            return Err!(Request(Forbidden("Server is not allowed to see event.")));
        }

        Ok(())
    }
}

pub(super) async fn require_known_room(
    services: &Services,
    room_id: &RoomId,
    origin: &ServerName,
) -> Result {
    if !services.rooms.metadata.exists(room_id).await {
        return Err!(Request(NotFound("Room is unknown to this server.")));
    }

    services
        .rooms
        .event_handler
        .acl_check(origin, room_id)
        .await
}

/// Access check shared by the endpoints that return the state or auth chain at
/// an event: the origin must be able to reach the room, and the event must be
/// in it.
pub(super) async fn check_event_in_room_access(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> Result {
    AccessCheck {
        services,
        origin,
        room_id,
        event_id: None,
    }
    .check()
    .await?;

    require_event_in_room(services, event_id, room_id).await
}

async fn require_event_in_room(
    services: &Services,
    event_id: &EventId,
    room_id: &RoomId,
) -> Result {
    services
        .rooms
        .timeline
        .get_pdu_json(event_id)
        .await
        .ok()
        .filter(|pdu| {
            pdu.get("room_id").and_then(CanonicalJsonValue::as_str) == Some(room_id.as_str())
        })
        .map(drop)
        .ok_or_else(|| err!(Request(NotFound("Event not found."))))
}
