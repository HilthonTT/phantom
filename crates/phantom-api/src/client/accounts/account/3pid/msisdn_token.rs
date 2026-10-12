use std::{net::IpAddr, time::Duration};

use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::{Err, Result, err};
use phantom_service::{Services, auth::threepid::canonicalize_msisdn};
use ruma::{OwnedSessionId, thirdparty::Medium};
use serde::Deserialize;
use serde_json::json;

use crate::router::ClientIp;

/// Lifetime of a pending phone verification before it self-reaps.
const PENDING_TTL: Duration = Duration::from_mins(15);

/// Where clients send the code the user read off the text message.
const SUBMIT_PATH: &str = "/_phantom/3pid/msisdn/submitToken";

/// The session a phone number's requestToken opened, and where its code goes.
pub(super) struct MsisdnSession {
    pub(super) sid: OwnedSessionId,
    pub(super) submit_url: String,
}

/// Refuses a phone request when no SMS gateway is configured, before the
/// number is even read.
pub(super) fn require_sms(services: &Services) -> Result {
    if !services.sms.is_enabled() {
        return Err!(Request(ThreepidDenied(
            "Phone number verification is not configured"
        )));
    }

    Ok(())
}

/// The msisdn a client's country and typed number stand for.
pub(super) fn msisdn(country: &str, phone_number: &str) -> Result<String> {
    canonicalize_msisdn(country, phone_number)
}

/// The requestToken spine for the msisdn medium, as `send_email_token` is for
/// email: throttle, open or reuse a pending verification, and text the code
/// when one is freshly minted. The caller checks whether the number may be
/// used beforehand.
pub(super) async fn send_msisdn_token(
    services: &Services,
    client: IpAddr,
    client_secret: &str,
    msisdn: &str,
    send_attempt: u64,
) -> Result<MsisdnSession> {
    let submit_url = submit_url(services)?;

    services.threepid.check_ip_rate_limit(client)?;
    services.threepid.check_address_rate_limit(msisdn)?;

    let outcome = services
        .threepid
        .create_or_reuse_pending(
            client_secret,
            Medium::Msisdn,
            msisdn,
            send_attempt,
            PENDING_TTL,
        )
        .await?;

    if let Some(code) = outcome.freshly_minted_token {
        services.sms.send_code(msisdn, &code).await?;
    }

    let sid = outcome
        .sid
        .parse()
        .map_err(|_| err!("Generated an invalid session id"))?;

    Ok(MsisdnSession { sid, submit_url })
}

fn submit_url(services: &Services) -> Result<String> {
    let base = services
        .config
        .auth
        .well_known_client
        .as_ref()
        .map(ToString::to_string)
        .ok_or_else(|| {
            err!(Config(
                "well_known_client",
                "A public client base URL must be set to verify phone numbers"
            ))
        })?;

    Ok(format!("{}{SUBMIT_PATH}", base.trim_end_matches('/')))
}

#[derive(Deserialize)]
pub(crate) struct SubmitToken {
    sid: String,
    client_secret: String,
    token: String,
}

/// # `POST /_phantom/3pid/msisdn/submitToken`
///
/// The `submit_url` of phone verifications: the client posts the code the
/// user typed, and a match validates the session.
pub(crate) async fn submit_msisdn_token_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    Json(body): Json<SubmitToken>,
) -> Result<impl IntoResponse> {
    services.threepid.check_ip_rate_limit(client)?;

    services
        .threepid
        .validate_pending_token(&body.sid, &body.client_secret, body.token.trim())
        .await?;

    Ok(Json(json!({ "success": true })))
}
