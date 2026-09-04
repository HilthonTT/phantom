use std::{fmt::Debug, mem};

use bytes::BytesMut;
use phantom_core::{Err, Result, debug_error, err, http, text, trace, warn};
use reqwest::Client;
use ruma::api::{
    IncomingResponse, Metadata, OutgoingRequest,
    appservice::Registration,
    auth_scheme::{AccessToken, SendAccessToken},
    path_builder::SinglePath,
};

/// Sends a request to an appservice
///
/// Only returns Ok(None) if there is no url specified in the appservice
/// registration file
pub(crate) async fn send_request<T>(
    client: &Client,
    registration: Registration,
    request: T,
) -> Result<Option<T::IncomingResponse>>
where
    T: OutgoingRequest
        + Metadata<Authentication = AccessToken, PathBuilder = SinglePath>
        + Debug
        + Send,
{
    let Some(dest) = registration.url else {
        return Ok(None);
    };

    if dest == "null" || dest.is_empty() {
        return Ok(None);
    }

    trace!(
        "Appservice URL \"{dest}\", Appservice ID: {}",
        registration.id
    );

    let hs_token = registration.hs_token.as_str();
    let mut http_request = request
        .try_into_http_request::<BytesMut>(&dest, SendAccessToken::IfRequired(hs_token), ())
        .map_err(|e| {
            err!(BadServerResponse(warn!(
                message = format_args!("Failed to find destination {dest}: {e:?}"),
                appservice = %registration.id,
            )))
        })?
        .map(BytesMut::freeze);

    let mut parts = http_request.uri().clone().into_parts();
    let old_path_and_query = parts.path_and_query.unwrap().as_str().to_owned();
    let symbol = if old_path_and_query.contains('?') {
        "&"
    } else {
        "?"
    };

    parts.path_and_query = Some(
        (old_path_and_query + symbol + "access_token=" + hs_token)
            .parse()
            .unwrap(),
    );
    *http_request.uri_mut() = parts.try_into().expect("our manipulation is always valid");

    let reqwest_request = reqwest::Request::try_from(http_request)?;

    let mut response = client.execute(reqwest_request).await.map_err(|e| {
        warn!(
            "Could not send request to appservice \"{}\" at {dest}:{e:?}",
            registration.id
        );
        e
    })?;

    let status = response.status();
    let mut http_response_builder = http::Response::builder()
        .status(status)
        .version(response.version());

    mem::swap(
        response.headers_mut(),
        http_response_builder
            .headers_mut()
            .expect("http::response::Builder is usable"),
    );

    let body = response.bytes().await?; // TODO: Handle timeouts and other errors more gracefully

    if !status.is_success() {
        debug_error!(
            "Appservice response bytes: {:?}",
            text::string_from_bytes(&body)
        );
        return Err!(BadServerResponse(warn!(
            "Appservice \"{}\" returned unsuccessful HTTP response {status} at {dest}",
            registration.id
        )));
    }

    let response = T::IncomingResponse::try_from_http_response(
        http_response_builder
            .body(body)
            .expect("reqwest body is valid http body"),
    );

    response.map(Some).map_err(|e| {
        err!(BadServerResponse(warn!(
            "Appservice \"{}\" returned invalid/malformed response bytes {dest}: {e}",
            registration.id
        )))
    })
}
