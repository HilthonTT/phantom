mod claim_keys;
mod get_key_changes;
mod get_keys;
mod upload_keys;
mod upload_signatures;
mod upload_signing_keys;

use std::{collections::BTreeMap, time::Duration};

use phantom_core::Result;
use phantom_service::Services;
use ruma::{
    OwnedServerName, ServerName,
    api::{
        Metadata, OutgoingRequest, federation::authentication::ServerSignatures,
        path_builder::SinglePath,
    },
};
use serde_json::{Value as JsonValue, json};

pub(crate) use self::{
    claim_keys::claim_keys_route, get_key_changes::get_key_changes_route, get_keys::get_keys_route,
    upload_keys::upload_keys_route, upload_signatures::upload_signatures_route,
    upload_signing_keys::upload_signing_keys_route,
};

/// Per-server errors reported back to the client in a key query or claim.
type Failures = BTreeMap<String, JsonValue>;

/// Sends one key request to a remote server, bounded by
/// `federation_keys_timeout` so an unresponsive server cannot outlast the
/// client's own request.
async fn execute_keys<T>(
    services: &Services,
    server: &ServerName,
    request: T,
) -> Result<T::IncomingResponse>
where
    T: OutgoingRequest + Metadata<Authentication = ServerSignatures, PathBuilder = SinglePath>,
    T: std::fmt::Debug + Send,
{
    let timeout = Duration::from_secs(services.config.client.federation_keys_timeout);

    tokio::time::timeout(timeout, services.federation.execute(server, request))
        .await
        .map_err(|_| phantom_core::err!(Request(Unknown("Timed out waiting for {server}"))))?
}

/// Records a remote server's failure in the response's `failures` map.
fn record_failure(failures: &mut Failures, server: &OwnedServerName, error: &phantom_core::Error) {
    failures.insert(
        server.to_string(),
        json!({ "status": 503, "message": error.to_string() }),
    );
}
