use std::fmt::Debug;

use phantom_core::Result;
use reqwest::Client;
use ruma::api::{OutgoingRequest, appservice::Registration};

#[expect(dead_code, reason = "the sender that calls this is not written yet")]
pub(crate) async fn send_request<T>(
    _client: &Client,
    registration: Registration,
    _request: T,
) -> Result<Option<T::IncomingResponse>>
where
    T: OutgoingRequest + Debug + Send,
{
    let Some(dest) = registration.url else {
        return Ok(None);
    };

    if dest == "null" || dest.is_empty() {
        return Ok(None);
    }

    Ok(None)
}
