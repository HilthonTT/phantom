use std::{fmt::Debug, mem};

use bytes::Bytes;
use ipaddress::IPAddress;
use phantom_core::{
    Err, Error, Result, debug, debug_error, debug_warn, err, error::inspect_debug_log, http,
    implement, log::INFO_SPAN_LEVEL, trace,
};
use reqwest::{Client, Method, Request, Response, Url};
use ruma::{
    ServerName,
    api::{
        EndpointError, IncomingResponseExt, Metadata, OutgoingRequest, OutgoingRequestExt,
        auth_scheme::NoAuthentication,
        error::{Error as RumaError, ErrorBody},
        federation::authentication::{ServerSignatures, XMatrixSigningInput},
        path_builder::SinglePath,
    },
};

use super::peer::{Classification, classify_error};
use crate::{moderation::Restriction, resolver::lookup::ResolvedDest};

type FixedPath = SinglePath;

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "request", level = "debug")]
pub async fn execute<T>(&self, dest: &ServerName, request: T) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = ServerSignatures, PathBuilder = FixedPath>,
    T: Debug + Send,
{
    let client = &self.services.client.federation;

    self.execute_with(client, dest, request).await
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "request", level = "debug")]
pub async fn execute_with<T>(
    &self,
    client: &Client,
    dest: &ServerName,
    request: T,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = ServerSignatures, PathBuilder = FixedPath>,
    T: Debug + Send,
{
    let result = self.execute_uncounted(client, dest, request).await;

    self.record_outcome(dest, verdict(&result)).await;

    result
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "request", level = "debug")]
pub(super) async fn execute_uncounted<T>(
    &self,
    client: &Client,
    dest: &ServerName,
    request: T,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = ServerSignatures, PathBuilder = FixedPath>,
    T: Debug + Send,
{
    let origin = self.services.server.name.clone();
    let keypair = self.services.server_keys.keypair();
    let input = XMatrixSigningInput::new(origin, dest.to_owned(), keypair);

    self.execute_on(client, dest, request, input).await
}

fn verdict<T>(result: &Result<T>) -> Option<Option<Classification>> {
    result.as_ref().err().map(classify_error)
}

#[implement(super::Service)]
async fn record_outcome(&self, dest: &ServerName, verdict: Option<Option<Classification>>) {
    match verdict {
        None => self.record_success(dest).await,
        Some(Some(class)) => self.record_failure(dest, class),
        Some(None) => (),
    }
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "request", level = "debug")]
pub async fn execute_unsigned<T>(
    &self,
    dest: &ServerName,
    request: T,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = NoAuthentication, PathBuilder = FixedPath>,
    T: Debug + Send,
{
    let client = &self.services.client.federation;
    let result = self.execute_on(client, dest, request, ()).await;

    self.record_outcome(dest, verdict(&result)).await;

    result
}

#[implement(super::Service)]
#[tracing::instrument(skip_all, name = "synapse", level = "debug")]
pub async fn execute_unsigned_synapse<T>(
    &self,
    dest: &ServerName,
    request: T,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = NoAuthentication, PathBuilder = FixedPath>,
    T: Debug + Send,
{
    let client = &self.services.client.synapse;
    let result = self.execute_on(client, dest, request, ()).await;

    self.record_outcome(dest, verdict(&result)).await;

    result
}

#[implement(super::Service)]
#[tracing::instrument(name = "fed", level = INFO_SPAN_LEVEL, skip(self, client, request, auth))]
async fn execute_on<T>(
    &self,
    client: &Client,
    dest: &ServerName,
    request: T,
    auth: <T::Authentication as ruma::api::auth_scheme::AuthScheme>::Input<'_>,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<PathBuilder = FixedPath> + Send,
{
    if !self.services.server.config.federation.allow_federation {
        return Err!(Config("allow_federation", "Federation is disabled."));
    }

    if self
        .services
        .moderation
        .forbids(dest, Restriction::Federation)
    {
        return Err!(Request(Forbidden(debug_warn!(
            "Federation with {dest} is not allowed."
        ))));
    }

    let actual = self.services.resolver.actual_destination(dest).await?;

    let request = request
        .try_into_http_request::<Vec<u8>>(actual.string().as_str(), auth, ())
        .map_err(|e| err!(BadServerResponse("Invalid destination: {e:?}")))?;

    let request = self.prepare(request)?;

    self.perform::<T>(dest, &actual, request, client).await
}

#[implement(super::Service)]
async fn perform<T>(
    &self,
    dest: &ServerName,
    actual: &ResolvedDest,
    request: Request,
    client: &Client,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Send,
{
    let url = request.url().clone();
    let method = request.method().clone();

    debug!(?method, ?url, "Sending request");
    match client.execute(request).await {
        Ok(response) => handle_response::<T>(dest, actual, &method, &url, response)
            .await
            .inspect_err(|error| self.evict_misrouted(dest, actual, error)),
        Err(error) => {
            self.evict_route(dest, actual);

            Err(handle_error(actual, &method, &url, error).expect_err("always returns error"))
        }
    }
}

#[implement(super::Service)]
fn evict_misrouted(&self, dest: &ServerName, actual: &ResolvedDest, error: &Error) {
    let Error::Federation(_, response) = error else {
        return;
    };

    if matches!(response.body, ErrorBody::NotJson { .. }) {
        self.evict_route(dest, actual);
    }
}

#[implement(super::Service)]
fn evict_route(&self, dest: &ServerName, actual: &ResolvedDest) {
    let cache = &self.services.resolver.cache;

    cache.del_destination(dest).ok();
    cache.del_override(&actual.dest.hostname()).ok();
}

#[implement(super::Service)]
fn prepare(&self, request: http::Request<Vec<u8>>) -> Result<Request> {
    let request = Request::try_from(request)?;

    self.validate_url(request.url())?;
    self.services.server.check_running()?;

    Ok(request)
}

#[implement(super::Service)]
fn validate_url(&self, url: &Url) -> Result<()> {
    if let Some(url_host) = url.host_str()
        && let Ok(ip) = IPAddress::parse(url_host.trim_start_matches('[').trim_end_matches(']'))
    {
        trace!("Checking request URL IP {ip:?}");
        self.services.resolver.validate_ip(&ip)?;
    }

    Ok(())
}

async fn handle_response<T>(
    dest: &ServerName,
    actual: &ResolvedDest,
    method: &Method,
    url: &Url,
    response: Response,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Send,
{
    let response = into_http_response(dest, actual, method, url, response).await?;
    let (parts, body) = response.into_parts();

    T::IncomingResponse::try_from_http_response(http::Response::from_parts(parts, &body[..]))
        .map_err(|e| err!(BadServerResponse("Server returned bad 200 response: {e:?}")))
}

async fn into_http_response(
    dest: &ServerName,
    actual: &ResolvedDest,
    method: &Method,
    url: &Url,
    mut response: Response,
) -> Result<http::Response<Bytes>> {
    let status = response.status();
    trace!(
        ?status, ?method,
        request_url = ?url,
        response_url = ?response.url(),
        "Received response from {}",
        actual.string(),
    );

    let mut http_response_builder = http::Response::builder()
        .status(status)
        .version(response.version());

    mem::swap(
        response.headers_mut(),
        http_response_builder
            .headers_mut()
            .expect("http::response::Builder is usable"),
    );

    trace!("Waiting for response body...");
    let body = response
        .bytes()
        .await
        .inspect_err(inspect_debug_log)
        .unwrap_or_else(|_| Vec::new().into());

    let http_response = http_response_builder
        .body(body)
        .expect("reqwest body is valid http body");

    debug!("Got {status:?} for {method} {url}");
    if !status.is_success() {
        let (parts, body) = http_response.into_parts();

        return Err(Error::Federation(
            dest.to_owned(),
            RumaError::from_http_response(http::Response::from_parts(parts, &body[..])),
        ));
    }

    Ok(http_response)
}

fn handle_error(
    actual: &ResolvedDest,
    method: &Method,
    url: &Url,
    mut e: reqwest::Error,
) -> Result {
    if e.is_timeout() || e.is_connect() {
        e = e.without_url();
        debug_warn!("{e:?}");
    } else if e.is_redirect() {
        debug_error!(
            method = ?method,
            url = ?url,
            final_url = ?e.url(),
            "Redirect loop {}: {}",
            actual.host,
            e,
        );
    } else {
        debug_error!("{e:?}");
    }

    Err(e.into())
}
