use axum::extract::State;
use futures::FutureExt;
use ruma::api::client::membership::ban_user;
use phantom_core::{Err, Result};

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/rooms/{roomId}/ban`
///
/// Tries to send a ban event into the room.
pub(crate) async fn ban_user_route(
    State(services): State<crate::router::State>,
    body: Ruma<ban_user::v3::Request>,
) -> Result<ban_user::v3::Response> {
    let sender_user = body.sender_user();

    if sender_user == body.user_id {
        return Err!(Request(Forbidden("You cannot ban yourself.")));
    }

    let state_lock = services.rooms.state.mutex.lock(&body.room_id).await;

    services
        .rooms
        .membership
        .ban(&body.room_id, &body.user_id, body.reason.as_ref(), sender_user, &state_lock)
        .boxed()
        .await?;

    drop(state_lock);

    Ok(ban_user::v3::Response::new())
}
