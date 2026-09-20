use futures::StreamExt;
use phantom_core::{error, implement, stream::ReadyExt};
use ruma::{EventId, RoomId, ServerName, events::room::history_visibility::HistoryVisibility};

#[implement(super::Service)]
#[tracing::instrument(skip_all, level = "trace")]
pub async fn server_can_see_event(
    &self,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
) -> bool {
    let Ok(shortstatehash) = self.pdu_shortstatehash(event_id).await else {
        return true;
    };

    let history_visibility = self.history_visibility_at(shortstatehash).await;

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
        HistoryVisibility::WorldReadable | HistoryVisibility::Shared => true,
        _ => {
            error!(
                %room_id,
                %origin,
                ?history_visibility,
                "Unknown history visibility; refusing to share the event",
            );

            false
        }
    }
}
