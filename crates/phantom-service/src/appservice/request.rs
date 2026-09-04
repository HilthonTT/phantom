//! Sending a request to an appservice.
//!
//! An appservice is reached at the URL its registration names, authenticated
//! with the `hs_token` that registration also carries — this server proving
//! itself to the appservice, the opposite direction from the `as_token` an
//! appservice proves itself to us with.

use std::{fmt::Debug, mem};

use bytes::{Bytes, BytesMut};
use phantom_core::{Err, Result, debug_error, err, http, implement, text, trace, warn};
use ruma::api::{
    IncomingResponse, Metadata, OutgoingRequest,
    appservice::Registration,
    auth_scheme::{AccessToken, SendAccessToken},
    path_builder::SinglePath,
};

/// Sends one request to an appservice.
///
/// `Ok(None)` where the registration names no URL, which is how an appservice
/// that only polls this server rather than being pushed to is written: it is
/// registered, it is simply not somewhere requests go.
#[implement(super::Service)]
pub(crate) async fn send_request<T>(
    &self,
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

    add_access_token_query(&mut http_request, hs_token);

    let reqwest_request = reqwest::Request::try_from(http_request)?;

    let mut response = self
        .services
        .client
        .appservice
        .execute(reqwest_request)
        .await
        .map_err(|e| {
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

/// Appends the `hs_token` as an `access_token` query parameter.
///
/// The header the request already carries is the current scheme; this is the
/// deprecated one, and it goes out as well because appservices written against
/// the older spec never look at the header.
fn add_access_token_query(request: &mut http::Request<Bytes>, hs_token: &str) {
    let mut parts = request.uri().clone().into_parts();
    let old_path_and_query = parts
        .path_and_query
        .expect("request built by ruma always has a path")
        .as_str()
        .to_owned();

    let symbol = if old_path_and_query.contains('?') {
        "&"
    } else {
        "?"
    };

    parts.path_and_query = Some(
        format!("{old_path_and_query}{symbol}access_token={hs_token}")
            .parse()
            .expect("a valid path and query stays valid with a query parameter appended"),
    );

    *request.uri_mut() = parts.try_into().expect("our manipulation is always valid");
}

#[cfg(test)]
mod tests {
    use super::{Bytes, add_access_token_query, http};

    fn uri_after(uri: &str) -> String {
        let mut request = http::Request::builder()
            .uri(uri)
            .body(Bytes::new())
            .expect("valid request");

        add_access_token_query(&mut request, "hs_tok");

        request.uri().to_string()
    }

    /// A path with nothing after it opens the query string.
    #[test]
    fn the_token_opens_a_query_that_is_not_there() {
        assert_eq!(
            uri_after("http://as.example/_matrix/app/v1/transactions/1"),
            "http://as.example/_matrix/app/v1/transactions/1?access_token=hs_tok"
        );
    }

    /// A path that already carries parameters is appended to, not overwritten:
    /// ruma puts the request's own parameters there.
    #[test]
    fn the_token_joins_a_query_that_is() {
        assert_eq!(
            uri_after("http://as.example/_matrix/app/v1/rooms?limit=5"),
            "http://as.example/_matrix/app/v1/rooms?limit=5&access_token=hs_tok"
        );
    }

    /// The rest of the URI is left alone, which is what makes the appservice
    /// receive the request it would have without this.
    #[test]
    fn nothing_but_the_query_changes() {
        let out = uri_after("https://as.example:8448/prefix/_matrix/app/v1/ping");

        assert!(
            out.starts_with("https://as.example:8448/prefix/_matrix/app/v1/ping?"),
            "{out}"
        );
    }
}
