use std::{fmt::Debug, mem};

use bytes::BytesMut;
use phantom_core::Result;
use reqwest::Client;
use ruma::api::{
    IncomingResponse, MatrixVersion, OutgoingRequest, appservice::Registration,
    client::typing::create_typing_event::v3::Typing::No,
};

pub(crate) async fn send_request<T>(
    client: &Client,
    registration: Registration,
    request: T,
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
