use axum::extract::State;
use futures::FutureExt;
use phantom_core::Result;
use ruma::api::client::membership::unban_user;

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/rooms/{roomId}/unban`
///
/// Tries to send an unban event into the room.
pub(crate) async fn unban_user_route(
    State(services): State<crate::router::State>,
    body: Ruma<unban_user::v3::Request>,
) -> Result<unban_user::v3::Response> {
    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    services
        .rooms
        .membership
        .unban(
            &body.room_id,
            &body.user_id,
            body.reason.as_ref(),
            body.sender_user(),
            &state_lock,
        )
        .boxed()
        .await?;

    drop(state_lock);

    Ok(unban_user::v3::Response::new())
}
