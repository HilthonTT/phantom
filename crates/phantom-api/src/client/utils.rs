use phantom_core::{Err, Error, Result, http::StatusCode, warn};
use phantom_service::Services;
use ruma::{
    RoomId, UserId,
    api::error::{ErrorKind, SenderIgnoredErrorData},
    presence::PresenceState,
};

use crate::router::Ruma;

pub(crate) async fn invite_check(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
) -> Result {
    if services.config.membership.block_non_admin_invites
        && !services.admin.user_is_admin(sender_user).await
    {
        warn!("{sender_user} is not an admin and attempted to send an invite to {room_id}");
        return Err!(Request(Forbidden(
            "Invites are not allowed on this server."
        )));
    }

    Ok(())
}

/// Whether the caller may change display names under `enable_set_displayname`.
///
/// Appservices and server admins are exempt, matching Synapse's exemption for
/// admins. `is_admin` is awaited only when the option is off and the caller
/// is not an appservice.
pub(crate) async fn may_set_displayname<T>(
    services: &Services,
    body: &Ruma<T>,
    is_admin: impl AsyncFnOnce() -> bool,
) -> bool
where
    T: Sync,
{
    services.config.client.enable_set_displayname
        || body.appservice_info.is_some()
        || is_admin().await
}

/// Marks a local user online after client activity.
///
/// Appservice traffic is not user activity, and nothing is recorded while
/// local presence is disabled.
pub(crate) async fn ping_presence<T>(
    services: &Services,
    body: &Ruma<T>,
    user_id: &UserId,
) -> Result
where
    T: Sync,
{
    if !services.config.client.allow_local_presence || body.appservice_info.is_some() {
        return Ok(());
    }

    services
        .presence
        .ping_presence(user_id, &PresenceState::Online)
        .await
}

/// The `M_SENDER_IGNORED` error for an event whose sender the caller ignores.
pub(crate) fn sender_ignored(sender: &UserId) -> Error {
    Error::Request(
        ErrorKind::SenderIgnored(SenderIgnoredErrorData::with_sender(sender.to_owned())),
        "You have ignored the user that sent this event".into(),
        StatusCode::NOT_FOUND,
    )
}
