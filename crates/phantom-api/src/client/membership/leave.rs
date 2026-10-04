use axum::extract::State;
use futures::{FutureExt, StreamExt};
use phantom_core::{Result, debug, trace};
use ruma::api::client::membership::leave_room::v3::{Request, Response};

use crate::router::Ruma;

/// Leaves a room through `POST /_matrix/client/v3/rooms/{roomId}/leave`.
#[tracing::instrument(level = "debug", skip_all)]
pub(crate) async fn leave_room_route(
    State(services): State<crate::router::State>,
    Ruma {
        body, sender_user, ..
    }: Ruma<Request>,
) -> Result<Response> {
    let services = &*services;
    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;
    let sender = sender_user
        .as_deref()
        .expect("user must be authenticated for this handler");

    services
        .rooms
        .membership
        .leave(sender, &body.room_id, body.reason, false, &state_lock)
        .await?;

    if !services.config.client.delete_rooms_after_leave {
        return Ok(Response::new());
    }

    let has_local_users = services
        .rooms
        .state_cache
        .local_users_in_room(&body.room_id)
        .boxed()
        .next()
        .await
        .is_some();

    let has_local_invites = services
        .rooms
        .state_cache
        .room_members_invited(&body.room_id)
        .any(|user_id| async move { services.server_state.user_is_local(user_id) })
        .await;

    if has_local_users || has_local_invites {
        trace!(room_id = %body.room_id, "Not deleting with local joined or invited");
        return Ok(Response::new());
    }

    debug!(room_id = %body.room_id, "Deleting room left empty by its last local member");
    services
        .rooms
        .delete
        .delete_room(&body.room_id, false, &state_lock)
        .boxed()
        .await?;

    Ok(Response::new())
}
