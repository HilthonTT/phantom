use axum::extract::State;
use phantom_core::Result;
use ruma::api::client::push::get_pushers;

use crate::router::Ruma;

/// # `GET /_matrix/client/r0/pushers`
///
/// Gets all currently active pushers for the sender user.
pub(crate) async fn get_pushers_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_pushers::v3::Request>,
) -> Result<get_pushers::v3::Response> {
    let sender_user = body.sender_user();

    Ok(get_pushers::v3::Response::new(
        services.pusher.get_pushers(sender_user).await,
    ))
}
