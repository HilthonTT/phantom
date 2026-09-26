use phantom_core::{Err, Result, debug_error, err, warn};
use phantom_service::{Services, net::server_keys::PubKeyMap, ops::moderation::Restriction};
use ruma::api::federation::authentication::{XMatrix, XMatrixVerificationError};

use super::{Auth, RawRequest};

pub(super) async fn authenticate(services: &Services, request: &RawRequest) -> Result<Auth> {
    if !services.server.config.federation.allow_federation {
        return Err!(Config("allow_federation", "Federation is disabled."));
    }

    let x_matrix = XMatrix::extract_from_http_headers(&request.parts.headers).map_err(|e| {
        err!(Request(Forbidden(debug_warn!(
            "Invalid X-Matrix authorization: {e}"
        ))))
    })?;

    let origin = &x_matrix.origin;

    if services.moderation.forbids(origin, Restriction::Federation) {
        return Err!(Request(Forbidden(debug_warn!(
            "Federation requests from {origin} denied."
        ))));
    }

    let verify_key = services
        .server_keys
        .get_verify_key(origin, &x_matrix.key)
        .await
        .map_err(|e| {
            err!(Request(Forbidden(debug_warn!(
                "Failed to fetch signing keys: {e}"
            ))))
        })?;

    let public_keys: PubKeyMap = [(
        origin.to_string(),
        [(x_matrix.key.to_string(), verify_key.key)].into(),
    )]
    .into();

    let mut signed_request = http::Request::new(request.body.as_ref());
    *signed_request.method_mut() = request.parts.method.clone();
    *signed_request.uri_mut() = request.parts.uri.clone();

    let destination = services.server_state.server_name();

    match x_matrix.verify_http_request(&signed_request, destination, &public_keys) {
        Ok(()) => Ok(Auth::server(origin.clone())),
        Err(XMatrixVerificationError::DestinationMismatch) => {
            Err!(Request(Unauthorized("Invalid destination.")))
        }
        Err(e) => {
            debug_error!("Failed to verify federation request from {origin}: {e}");

            if request.parts.uri.path().contains('@') {
                warn!(
                    "Request URI contained '@'. Make sure your reverse proxy passes the raw URI \
                     to phantom (apache: use nocanon)"
                );
            }

            Err!(Request(Forbidden("Failed to verify X-Matrix signatures.")))
        }
    }
}
