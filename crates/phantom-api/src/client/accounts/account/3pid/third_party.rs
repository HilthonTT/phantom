use axum::extract::State;
use futures::StreamExt;
use phantom_core::Result;
use ruma::api::client::account::get_3pids::{self, v3::Response};

use crate::router::Ruma;

/// # `GET _matrix/client/v3/account/3pid`
///
/// Get the third party identifiers bound to this account.
pub(crate) async fn third_party_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_3pids::v3::Request>,
) -> Result<Response> {
    let threepids = services
        .threepid
        .get_bindings(body.sender_user())
        .collect()
        .await;

    Ok(Response::new(threepids))
}
