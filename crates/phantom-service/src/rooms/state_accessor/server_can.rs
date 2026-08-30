//! What a remote server is allowed to see in a room.
//!
//! The server-side counterpart to [`user_can`](super::user_can): a server may
//! see an event if any of its users could have, since anything sent to one of
//! them has reached that server anyway.

use futures::StreamExt;
use phantom_core::{implement, stream::ReadyExt};
use ruma::{
    EventId, RoomId, ServerName,
    events::{
        StateEventType,
        room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
    },
};

/// Whether `origin` may be sent `event_id`, by the room's `history_visibility`
/// at that event.
#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn server_can_see_event(
    &self,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> bool {
    // See the matching note in `user_can_see_event`: an event with no
    // recorded state is not judged.
    let Ok(shortstatehash) = self.pdu_shortstatehash(event_id).await else {
        return true;
    };

    let history_visibility = self
        .state_get_content(shortstatehash, &StateEventType::RoomHistoryVisibility, "")
        .await
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        );

    // Current members rather than members at that state: a server that has a
    // user in the room now is one this server is already talking to about it.
    let current_server_members = self
        .services
        .state_cache
        .room_members(room_id)
        .ready_filter(|member| member.server_name() == origin);

    match history_visibility {
        HistoryVisibility::Invited => {
            current_server_members
                .any(|member| self.user_was_invited(shortstatehash, member))
                .await
        }
        HistoryVisibility::Joined => {
            current_server_members
                .any(|member| self.user_was_joined(shortstatehash, member))
                .await
        }
        HistoryVisibility::WorldReadable | HistoryVisibility::Shared | _ => true,
    }
}
