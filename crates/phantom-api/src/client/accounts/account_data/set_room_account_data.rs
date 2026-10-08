use axum::extract::State;
use phantom_core::Result;
use ruma::api::client::config::set_room_account_data;

use super::{assert_account_data_owner, set_account_data};
use crate::router::Ruma;

/// # `PUT /_matrix/client/r0/user/{userId}/rooms/{roomId}/account_data/{type}`
///
/// Sets some room account data for the sender user.
pub(crate) async fn set_room_account_data_route(
    State(services): State<crate::router::State>,
    body: Ruma<set_room_account_data::v3::Request>,
) -> Result<set_room_account_data::v3::Response> {
    let sender_user = body.sender_user();

    assert_account_data_owner(
        sender_user,
        &body.user_id,
        body.appservice_info.as_ref(),
        "You cannot set account data for other users.",
    )?;

    set_account_data(
        &services,
        Some(&body.room_id),
        &body.user_id,
        &body.event_type.to_string(),
        body.data.json(),
    )
    .await?;

    Ok(set_room_account_data::v3::Response::new())
}
