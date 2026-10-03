use axum::extract::State;
use phantom_core::{Err, Result, math::Tried};
use ruma::api::client::typing::create_typing_event;

use crate::{client::utils::ping_presence, router::Ruma};

/// # `PUT /_matrix/client/r0/rooms/{roomId}/typing/{userId}`
///
/// Sets the typing state of the sender user.
pub(crate) async fn create_typing_event_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_typing_event::v3::Request>,
) -> Result<create_typing_event::v3::Response> {
    use create_typing_event::v3::Typing;
    let sender_user = body.sender_user();

    if sender_user != body.user_id && body.appservice_info.is_none() {
        return Err!(Request(Forbidden(
            "You cannot update typing status of other users."
        )));
    }

    if !services
        .rooms
        .state_cache
        .is_joined(sender_user, &body.room_id)
        .await
    {
        return Err!(Request(Forbidden("You are not in this room.")));
    }

    match body.state {
        Typing::Yes(info) => {
            let duration = Ord::clamp(
                info.timeout.as_millis().try_into().unwrap_or(u64::MAX),
                services
                    .server
                    .config
                    .client
                    .typing_client_timeout_min_s
                    .try_mul(1000)?,
                services
                    .server
                    .config
                    .client
                    .typing_client_timeout_max_s
                    .try_mul(1000)?,
            );
            services
                .rooms
                .typing
                .typing_add(
                    sender_user,
                    &body.room_id,
                    phantom_core::time::now_millis()
                        .checked_add(duration)
                        .expect("user typing timeout should not get this high"),
                )
                .await?;
        }
        _ => {
            services
                .rooms
                .typing
                .typing_remove(sender_user, &body.room_id)
                .await?;
        }
    }

    ping_presence(&services, &body, &body.user_id).await?;

    Ok(create_typing_event::v3::Response::new())
}
