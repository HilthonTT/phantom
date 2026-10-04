use futures::future::join3;
use ruma::{
    RoomId, api::client::sync::sync_events::v5::request::ListFilters, directory::RoomTypeFilter,
    events::room::member::MembershipState,
};

use super::SyncInfo;

/// Whether a room passes a list's filters.
///
/// ruma's MSC4186 filters carry no `tags`, `not_tags` or `spaces`, so only the
/// direct, encryption, invite and room-type filters apply.
#[tracing::instrument(name = "filter", level = "trace", skip_all)]
pub(super) async fn filter_room(
    SyncInfo {
        services,
        sender_user,
        direct_rooms,
        ..
    }: SyncInfo<'_>,
    filter: &ListFilters,
    room_id: &RoomId,
    membership: Option<&MembershipState>,
) -> bool {
    let match_direct = filter
        .is_dm
        .is_none_or(|is_dm| is_dm == direct_rooms.contains(room_id));

    if !match_direct {
        return false;
    }

    #[expect(clippy::match_same_arms)] // helps readability
    let match_invite = async {
        match (membership, filter.is_invite) {
            (_, None) => true,
            (Some(MembershipState::Invite), Some(true)) => true,
            (Some(MembershipState::Invite), Some(false)) => false,
            (Some(_), Some(true)) => false,
            (Some(_), Some(false)) => true,
            (None, Some(is_invite)) => {
                services
                    .rooms
                    .state_cache
                    .is_invited(sender_user, room_id)
                    .await
                    == is_invite
            }
        }
    };

    let match_encrypted = async {
        match filter.is_encrypted {
            None => true,
            Some(is_encrypted) => {
                services
                    .rooms
                    .state_accessor
                    .is_encrypted_room(room_id)
                    .await
                    == is_encrypted
            }
        }
    };

    let match_room_type = async {
        if filter.room_types.is_empty() && filter.not_room_types.is_empty() {
            return true;
        }

        let room_type = services
            .rooms
            .state_accessor
            .get_room_type(room_id)
            .await
            .ok();

        let room_type = RoomTypeFilter::from(room_type);
        (filter.not_room_types.is_empty() || !filter.not_room_types.contains(&room_type))
            && (filter.room_types.is_empty() || filter.room_types.contains(&room_type))
    };

    let (invite, encrypted, room_type) =
        join3(match_invite, match_encrypted, match_room_type).await;

    invite && encrypted && room_type
}

/// Whether a subscribed room may be served at all: it exists, is neither
/// disabled nor banned, and the user can see it.
#[tracing::instrument(name = "filter_meta", level = "trace", skip_all)]
pub(super) async fn filter_room_meta(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    room_id: &RoomId,
) -> bool {
    let rooms = &services.rooms;

    if !rooms.metadata.exists(room_id).await
        || rooms.metadata.is_disabled(room_id).await
        || rooms.metadata.is_banned(room_id).await
    {
        return false;
    }

    rooms
        .state_accessor
        .user_can_see_state_events(sender_user, room_id)
        .await
        || rooms.state_cache.is_invited(sender_user, room_id).await
        || rooms.state_cache.once_joined(sender_user, room_id).await
}
