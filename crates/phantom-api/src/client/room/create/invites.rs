use futures::{FutureExt, StreamExt};
use phantom_core::{
    stream::{IterStream, ReadyExt},
    warn,
};
use phantom_service::Services;
use ruma::{
    RoomId, UserId,
    api::client::room::create_room,
    events::{
        GlobalAccountDataEventType,
        ignored_user_list::{IgnoredUserListEvent, IgnoredUserListEventContent},
    },
};

use crate::router::Ruma;

pub(super) async fn process_invites(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    sender_user: &UserId,
    room_id: &RoomId,
) {
    // 8. Events implied by invite (and TODO: invite_3pid)
    let ignored = ignored_users(services, sender_user).await;

    body.invite
        .iter()
        .stream()
        .ready_filter(|user_id| invite_allowed(ignored.as_ref(), user_id))
        .for_each(async |user_id| {
            if let Err(e) = services
                .rooms
                .membership
                .invite(sender_user, user_id, room_id, None, body.is_direct)
                .boxed()
                .await
            {
                warn!(%e, "Failed to send invite");
            }
        })
        .await;
}

pub(super) async fn ignored_users(
    services: &Services,
    user_id: &UserId,
) -> Option<IgnoredUserListEventContent> {
    services
        .account_data
        .get_global(user_id, GlobalAccountDataEventType::IgnoredUserList)
        .await
        .map(|ignored: IgnoredUserListEvent| ignored.content)
        .ok()
}

/// Gate an invitee against the sender's own ignore list.
///
/// The invitee's own invite permission is not consulted here: `local_invite`
/// is the authoritative check and refuses a blocked invite the same way any
/// other failed invite is handled, while an ignored one proceeds and is
/// withheld from the invitee afterwards.
pub(super) fn invite_allowed(
    ignored: Option<&IgnoredUserListEventContent>,
    invitee: &UserId,
) -> bool {
    ignored.is_none_or(|content| !content.ignored_users.contains_key(invitee))
}
