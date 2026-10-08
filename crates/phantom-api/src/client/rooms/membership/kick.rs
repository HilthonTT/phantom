use axum::extract::State;
use futures::FutureExt;
use phantom_core::{Err, Result};
use ruma::api::client::membership::kick_user;

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/rooms/{roomId}/kick`
///
/// Tries to send a kick event into the room.
pub(crate) async fn kick_user_route(
    State(services): State<crate::router::State>,
    body: Ruma<kick_user::v3::Request>,
) -> Result<kick_user::v3::Response> {
    let sender_user = body.sender_user();

    if sender_user == body.user_id {
        return Err!(Request(Forbidden("You cannot kick yourself.")));
    }

    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    services
        .rooms
        .membership
        .kick(
            &body.room_id,
            &body.user_id,
            body.reason.as_ref(),
            sender_user,
            &state_lock,
        )
        .boxed()
        .await?;

    drop(state_lock);

    Ok(kick_user::v3::Response::new())
}
