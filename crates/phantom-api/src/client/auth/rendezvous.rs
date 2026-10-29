//! Rendezvous sessions for signing in by QR code.
//!
//! Two devices pass encrypted messages through a session until the new one
//! has completed the OIDC device authorization grant. Two protocols share the
//! sessions: MSC4388 in JSON with a sequence token, and the MSC4108 it
//! replaces, which carries raw text and versions it with HTTP `ETag`s. ruma
//! has types for all of MSC4388 but only MSC4108's create, whose `text/plain`
//! body a `Ruma` extractor would refuse as not JSON, so MSC4108 is served by
//! hand.

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{
        CACHE_CONTROL, CONTENT_TYPE, ETAG, EXPIRES, IF_MATCH, IF_NONE_MATCH, LAST_MODIFIED, PRAGMA,
    },
};
use phantom_core::{Err, Error, Result, err};
use phantom_service::{
    Services,
    auth::rendezvous::{Get, Meta, Put},
};
use ruma::{
    UserId,
    api::{
        client::rendezvous::{
            create_rendezvous_session, delete_rendezvous_session, discover_rendezvous,
            get_rendezvous_session, update_rendezvous_session,
        },
        error::ErrorKind,
    },
    http_headers::system_time_to_http_date,
};
use serde_json::json;

use crate::router::{ClientIp, Ruma, optional_user};

/// MSC4108's session path, under which a created session's URL is handed out.
pub(crate) const MSC4108_PATH: &str = "/_matrix/client/unstable/org.matrix.msc4108/rendezvous";

/// # `GET /_matrix/client/unstable/io.element.msc4388/rendezvous`
///
/// Whether the requester may create a session.
pub(crate) async fn discover_rendezvous_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<discover_rendezvous::unstable::Request>,
) -> Result<discover_rendezvous::unstable::Response> {
    check(&services, client, None)?;

    Ok(discover_rendezvous::unstable::Response::new(may_create(
        &services,
        body.sender_user.as_deref(),
    )))
}

/// # `POST /_matrix/client/unstable/io.element.msc4388/rendezvous`
///
/// Creates a session holding `data`.
pub(crate) async fn create_rendezvous_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<create_rendezvous_session::unstable_msc4388::Request>,
) -> Result<create_rendezvous_session::unstable_msc4388::Response> {
    check(&services, client, Some(body.sender_user.as_deref()))?;
    check_size(&services, body.data.len())?;

    let (id, meta) = services
        .rendezvous
        .create(Bytes::copy_from_slice(body.data.as_bytes()));

    Ok(create_rendezvous_session::unstable_msc4388::Response::new(
        id.to_string(),
        meta.sequence_token().to_owned(),
        meta.expires_in(),
    ))
}

/// # `GET /_matrix/client/unstable/io.element.msc4388/rendezvous/{id}`
pub(crate) async fn get_rendezvous_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_rendezvous_session::unstable::Request>,
) -> Result<get_rendezvous_session::unstable::Response> {
    check(&services, client, None)?;

    let Get::Data { data, meta } = services.rendezvous.get(&body.id, None) else {
        return Err!(Request(NotFound("Rendezvous session not found.")));
    };

    Ok(get_rendezvous_session::unstable::Response::new(
        meta.sequence_token().to_owned(),
        String::from_utf8_lossy(&data).into_owned(),
        meta.expires_in(),
    ))
}

/// # `PUT /_matrix/client/unstable/io.element.msc4388/rendezvous/{id}`
///
/// Replaces the session's data if `sequence_token` is still current.
pub(crate) async fn update_rendezvous_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<update_rendezvous_session::unstable::Request>,
) -> Result<update_rendezvous_session::unstable::Response> {
    check(&services, client, None)?;
    check_size(&services, body.data.len())?;

    let data = Bytes::copy_from_slice(body.data.as_bytes());
    match services
        .rendezvous
        .put_token(&body.id, &body.sequence_token, data)
    {
        Put::Accepted(meta) => Ok(update_rendezvous_session::unstable::Response::new(
            meta.sequence_token().to_owned(),
        )),
        Put::PreconditionFailed(_) => Err(Error::Request(
            ErrorKind::ConcurrentWrite,
            "The session was written since that sequence token.".into(),
            StatusCode::CONFLICT,
        )),
        Put::NotFound => Err!(Request(NotFound("Rendezvous session not found."))),
    }
}

/// # `DELETE /_matrix/client/unstable/io.element.msc4388/rendezvous/{id}`
pub(crate) async fn delete_rendezvous_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<delete_rendezvous_session::unstable::Request>,
) -> Result<delete_rendezvous_session::unstable::Response> {
    check(&services, client, None)?;

    if !services.rendezvous.delete_if_active(&body.id) {
        return Err!(Request(NotFound("Rendezvous session not found.")));
    }

    Ok(delete_rendezvous_session::unstable::Response::new())
}

/// # `POST /_matrix/client/unstable/org.matrix.msc4108/rendezvous`
///
/// Creates a session holding the `text/plain` body and answers with its URL.
pub(crate) async fn msc4108_create_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let sender = optional_user(&services, &headers).await?;
    check(&services, client, Some(sender.as_deref()))?;
    check_size(&services, body.len())?;

    let is_text = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/plain"));

    if !is_text {
        return Err!(Request(InvalidParam("The body must be text/plain.")));
    }

    // The URL must be one the other device can reach, so it is built on the
    // client base URL; QR login needs the OIDC server, which needs it anyway.
    let Some(base) = services.config.auth.well_known_client.as_ref() else {
        return Err(unsupported(
            "QR-code login needs `well_known_client` to be set.",
        ));
    };

    let (id, meta) = services.rendezvous.create(body);

    let url = base
        .join(&format!("{}/{id}", MSC4108_PATH.trim_start_matches('/')))
        .map_err(|e| {
            err!(Config(
                "well_known_client",
                "cannot build a rendezvous URL: {e}"
            ))
        })?;

    Ok((
        StatusCode::OK,
        session_headers(&meta)?,
        Json(json!({ "url": url })),
    )
        .into_response())
}

/// # `GET /_matrix/client/unstable/org.matrix.msc4108/rendezvous/{id}`
pub(crate) async fn msc4108_get_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    check(&services, client, None)?;

    let if_none_match = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());

    match services.rendezvous.get(&id, if_none_match) {
        Get::Data { data, meta } => Ok((
            StatusCode::OK,
            session_headers(&meta)?,
            [(CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
            data,
        )
            .into_response()),
        Get::NotModified(meta) => {
            Ok((StatusCode::NOT_MODIFIED, session_headers(&meta)?).into_response())
        }
        Get::NotFound => Err!(Request(NotFound("Rendezvous session not found."))),
    }
}

/// # `PUT /_matrix/client/unstable/org.matrix.msc4108/rendezvous/{id}`
///
/// Replaces the session's data if `If-Match` names its current `ETag`.
pub(crate) async fn msc4108_put_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    check(&services, client, None)?;
    check_size(&services, body.len())?;

    let Some(if_match) = headers.get(IF_MATCH).and_then(|value| value.to_str().ok()) else {
        return Err!(Request(MissingParam("An If-Match header is required.")));
    };

    match services.rendezvous.put(&id, if_match, body) {
        Put::Accepted(meta) => Ok((StatusCode::ACCEPTED, session_headers(&meta)?).into_response()),
        // MSC4108's error code predates ruma's, which is MSC4388's.
        Put::PreconditionFailed(meta) => Ok((
            StatusCode::PRECONDITION_FAILED,
            session_headers(&meta)?,
            Json(json!({
                "errcode": "M_CONCURRENT_WRITE",
                "error": "The session was written since that ETag.",
            })),
        )
            .into_response()),
        Put::NotFound => Err!(Request(NotFound("Rendezvous session not found."))),
    }
}

/// # `DELETE /_matrix/client/unstable/org.matrix.msc4108/rendezvous/{id}`
pub(crate) async fn msc4108_delete_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    Path(id): Path<String>,
) -> Result<Response> {
    check(&services, client, None)?;

    if !services.rendezvous.delete_if_active(&id) {
        return Err!(Request(NotFound("Rendezvous session not found.")));
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Refuses every rendezvous request while the feature is off or the client is
/// over its rate limit. `creator` is `Some` for a request that creates a
/// session, holding the user who asked, if any.
fn check(
    services: &Services,
    client: std::net::IpAddr,
    creator: Option<Option<&UserId>>,
) -> Result {
    if !services.config.rendezvous.rendezvous_enabled {
        return Err(unsupported("QR-code login is disabled on this server."));
    }

    services.rendezvous.check_rate_limit(client)?;

    if let Some(user) = creator
        && !may_create(services, user)
    {
        return Err!(Request(Forbidden(
            "Only a signed-in device may create a rendezvous session."
        )));
    }

    Ok(())
}

fn may_create(services: &Services, user: Option<&UserId>) -> bool {
    let config = &services.config.rendezvous;

    config.rendezvous_enabled && (user.is_some() || !config.rendezvous_authenticated_only)
}

fn check_size(services: &Services, len: usize) -> Result {
    if len > services.config.rendezvous.rendezvous_session_max_bytes {
        return Err!(Request(TooLarge("The rendezvous message is too large.")));
    }

    Ok(())
}

/// Both MSCs answer a server without rendezvous with 404 `M_UNRECOGNIZED`,
/// which clients read as "QR login unavailable" rather than a failure.
fn unsupported(message: &'static str) -> Error {
    Error::Request(
        ErrorKind::Unrecognized,
        message.into(),
        StatusCode::NOT_FOUND,
    )
}

/// The headers MSC4108 sends with a session: its `ETag` and timestamps, and
/// no caching, since the data changes under the same URL.
fn session_headers(meta: &Meta) -> Result<[(http::HeaderName, HeaderValue); 5]> {
    let date =
        |time| system_time_to_http_date(&time).map_err(|e| err!("cannot format an HTTP date: {e}"));

    Ok([
        (
            ETAG,
            HeaderValue::from_str(meta.etag.as_str()).expect("an ETag is quoted base64url"),
        ),
        (EXPIRES, date(meta.expires_at)?),
        (LAST_MODIFIED, date(meta.last_modified)?),
        (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        (PRAGMA, HeaderValue::from_static("no-cache")),
    ])
}
