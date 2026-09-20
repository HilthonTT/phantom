use phantom_core::implement;
use ruma::{
    RoomId,
    events::{
        StateEventType,
        room::history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
    },
};

use crate::rooms::short::ShortStateHash;

/// A room's history visibility as of one point in its state.
///
/// A room with no `m.room.history_visibility` event, or one whose event will not
/// deserialize, is treated as `Shared` — the spec default, and the reading that
/// errs towards what members could already see rather than towards exposure.
#[implement(super::Service)]
pub(super) async fn history_visibility_at(
    &self,
    shortstatehash: ShortStateHash,
) -> HistoryVisibility {
    self.state_get_content(shortstatehash, &StateEventType::RoomHistoryVisibility, "")
        .await
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        )
}

/// A room's history visibility as of its current state.
///
/// Defaults exactly as [`Service::history_visibility_at`] does.
#[implement(super::Service)]
pub(super) async fn history_visibility_of(&self, room_id: &RoomId) -> HistoryVisibility {
    self.room_state_get_content(room_id, &StateEventType::RoomHistoryVisibility, "")
        .await
        .map_or(
            HistoryVisibility::Shared,
            |c: RoomHistoryVisibilityEventContent| c.history_visibility,
        )
}
