//! Clients registering themselves, per MSC2966 and RFC 7591.
//!
//! Next-gen auth has no out-of-band client registration: a Matrix client
//! arrives at a server it has never met and registers itself at the endpoint
//! this backs. Nothing authenticates that request, which is what shapes the
//! rules here — a registration is bounded in size, stripped of anything this
//! server does not implement, and identified by the hash of what it says, so
//! the same client registering twice gets the same `client_id` back instead of
//! filling the column with duplicates.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD as b64};
use phantom_core::{Err, Result, err, hash::sha256, implement, time::now_secs};
use phantom_database::{Cbor, Deserialized};
use serde::{Deserialize, Serialize};

/// Bounds one stored registration, so an unauthenticated endpoint cannot fill
/// the column with one enormous record.
const MAX_REGISTRATION_BYTES: usize = 4096;

/// What this server implements. MSC2966 requires anything else be dropped from
/// a registration before it is stored and echoed back, so that a client is
/// never told it may use a grant this server will refuse.
const KNOWN_GRANT_TYPES: [&str; 2] = ["authorization_code", "refresh_token"];
const KNOWN_RESPONSE_TYPES: [&str; 1] = ["code"];

/// A registration request, as the client sends it.
#[derive(Debug, Deserialize, Serialize)]
pub struct DcrRequest {
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    #[serde(default)]
    pub contacts: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Option<Vec<String>>,
    pub response_types: Option<Vec<String>>,
    pub application_type: Option<String>,
    pub policy_uri: Option<String>,
    pub tos_uri: Option<String>,
    pub software_id: Option<String>,
    pub software_version: Option<String>,
}

/// A registration as it is stored and echoed back, with the defaults filled in
/// and the unsupported types dropped.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientRegistration {
    pub client_id: String,
    pub redirect_uris: Vec<String>,
    pub client_name: Option<String>,
    pub client_uri: Option<String>,
    pub logo_uri: Option<String>,
    pub contacts: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub application_type: Option<String>,
    pub policy_uri: Option<String>,
    pub tos_uri: Option<String>,
    pub software_id: Option<String>,
    pub software_version: Option<String>,
    pub registered_at: u64,
}

/// Registers a client, or returns the registration it already has.
///
/// The `client_id` is the hash of the normalized request, so registering the
/// same metadata twice is idempotent — which matters because a client that
/// lost its own copy has no way to ask what its id was.
#[implement(super::Server)]
pub async fn register_client(&self, request: DcrRequest) -> Result<ClientRegistration> {
    let request = normalize(request);
    let serialized = serde_json::to_vec(&request).expect("a DcrRequest is always serializable");

    if serialized.len() > MAX_REGISTRATION_BYTES {
        return Err!(Request(TooLarge(
            "The client registration is over the {MAX_REGISTRATION_BYTES} byte limit"
        )));
    }

    let client_id = b64.encode(sha256::hash(&serialized));

    if let Ok(existing) = self.get_client(&client_id).await {
        return Ok(existing);
    }

    let registration = ClientRegistration {
        client_id,
        redirect_uris: request.redirect_uris,
        client_name: request.client_name,
        client_uri: request.client_uri,
        logo_uri: request.logo_uri,
        contacts: request.contacts,
        // A public client with no secret, which is what a Matrix client is.
        token_endpoint_auth_method: request
            .token_endpoint_auth_method
            .unwrap_or_else(|| "none".to_owned()),
        grant_types: request
            .grant_types
            .unwrap_or_else(|| vec!["authorization_code".to_owned(), "refresh_token".to_owned()]),
        response_types: request
            .response_types
            .unwrap_or_else(|| vec!["code".to_owned()]),
        application_type: request.application_type,
        policy_uri: request.policy_uri,
        tos_uri: request.tos_uri,
        software_id: request.software_id,
        software_version: request.software_version,
        registered_at: now_secs(),
    };

    self.db
        .oidcclientid_registration
        .raw_put(&registration.client_id, Cbor(&registration))?;

    Ok(registration)
}

/// The registration of that `client_id`.
#[implement(super::Server)]
pub async fn get_client(&self, client_id: &str) -> Result<ClientRegistration> {
    self.db
        .oidcclientid_registration
        .get(client_id)
        .await
        .deserialized::<Cbor<ClientRegistration>>()
        .map(|Cbor(registration)| registration)
        .map_err(|_| err!(Request(NotFound("Unknown client_id"))))
}

/// Puts a request into the one form its `client_id` is derived from.
///
/// Two registrations that mean the same thing have to hash the same, so the
/// lists are sorted and everything unsupported is dropped before the hash is
/// taken rather than after.
fn normalize(mut request: DcrRequest) -> DcrRequest {
    request.redirect_uris.sort();
    request.contacts.sort();
    prune(&mut request.grant_types, &KNOWN_GRANT_TYPES);
    prune(&mut request.response_types, &KNOWN_RESPONSE_TYPES);

    request
}

fn prune(types: &mut Option<Vec<String>>, known: &[&str]) {
    let Some(types) = types else {
        return;
    };

    types.retain(|ty| known.contains(&ty.as_str()));
    types.sort();
}
