use std::collections::HashSet;

use axum::extract::State;
use futures::StreamExt;
use phantom_core::{Result, err};
use ruma::api::client::keys::get_key_changes;

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/keys/changes`
///
/// Gets a list of users who have updated their device identity keys since the
/// previous sync token.
///
/// - TODO: left users
pub(crate) async fn get_key_changes_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_key_changes::v3::Request>,
) -> Result<get_key_changes::v3::Response> {
    let sender_user = body.sender_user();

    let from = body
        .from
        .parse()
        .map_err(|_| err!(Request(InvalidParam("Invalid `from`."))))?;

    let to = body
        .to
        .parse()
        .map_err(|_| err!(Request(InvalidParam("Invalid `to`."))))?;

    let mut changed: HashSet<_> = services
        .users
        .keys_changed(sender_user, from, Some(to))
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let mut rooms = services.rooms.state_cache.rooms_joined(sender_user).boxed();
    while let Some(room_id) = rooms.next().await {
        changed.extend(
            services
                .users
                .room_keys_changed(room_id, from, Some(to))
                .map(|(user_id, _)| user_id.to_owned())
                .collect::<Vec<_>>()
                .await,
        );
    }

    let mut response =
        get_key_changes::v3::Response::new(changed.into_iter().collect(), Vec::new());
    response.left = Vec::new();

    Ok(response)
}
