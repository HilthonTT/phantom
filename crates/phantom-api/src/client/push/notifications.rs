use axum::extract::State;
use phantom_core::{Result, err};
use ruma::api::client::push::get_notifications;

use crate::router::Ruma;

/// # `GET /_matrix/client/r0/notifications/`
///
/// Paginate through the list of events the user has been, or would have been
/// notified about.
///
/// Phantom keeps per-room notification counts but not the notified events
/// themselves, so the list is always empty.
pub(crate) async fn get_notifications_route(
    State(_services): State<crate::router::State>,
    body: Ruma<get_notifications::v3::Request>,
) -> Result<get_notifications::v3::Response> {
    body.body
        .from
        .as_deref()
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|e| err!(Request(InvalidParam("Invalid `from' parameter: {e}"))))?;

    Ok(get_notifications::v3::Response::new(Vec::new()))
}
