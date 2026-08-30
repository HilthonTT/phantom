//! Assembling the key map one verification needs.
//!
//! [`get_event_keys`](super::Service::get_event_keys) is the entry point: it
//! works out which keys an event has to be checked against and gathers
//! exactly those, fetching what is missing. Where a batch of events is about
//! to be verified, calling
//! [`acquire_events_pubkeys`](super::Service::acquire_events_pubkeys) over the
//! batch first is much cheaper than letting each event here fetch on its own.

use std::borrow::Borrow;

use phantom_core::{Err, Result, implement};
use ruma::{
    CanonicalJsonObject, RoomVersionId, ServerName, ServerSigningKeyId,
    api::federation::discovery::VerifyKey,
};

use super::{PubKeyMap, PubKeys, extract_key, required_keys};

#[implement(super::Service)]
pub async fn get_event_keys(
    &self,
    object: &CanonicalJsonObject,
    version: &RoomVersionId,
) -> Result<PubKeyMap> {
    let Some(rules) = version.rules() else {
        return Err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {version}"
        )));
    };

    let required = match required_keys(object, &rules.signatures) {
        Ok(required) => required,
        Err(e) => {
            return Err!(BadServerResponse(
                "Failed to determine keys required to verify: {e}"
            ));
        }
    };

    let batch = required
        .iter()
        .map(|(server, key_ids)| (server.borrow(), key_ids.iter().map(Borrow::borrow)));

    Ok(self.get_pubkeys(batch).await)
}

#[implement(super::Service)]
pub async fn get_pubkeys<'a, S, K>(&self, batch: S) -> PubKeyMap
where
    S: Iterator<Item = (&'a ServerName, K)> + Send,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut keys = PubKeyMap::new();
    for (server, key_ids) in batch {
        let pubkeys = self.get_pubkeys_for(server, key_ids).await;
        keys.insert(server.into(), pubkeys);
    }

    keys
}

/// The keys of `origin` named by `key_ids`, skipping any that could not be
/// obtained.
///
/// A key missing from the result is not an error here: an event only has to
/// carry one signature this server can check, so verification decides whether
/// what was gathered is enough.
#[implement(super::Service)]
pub async fn get_pubkeys_for<'a, I>(&self, origin: &ServerName, key_ids: I) -> PubKeys
where
    I: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut keys = PubKeys::new();
    for key_id in key_ids {
        if let Ok(verify_key) = self.get_verify_key(origin, key_id).await {
            keys.insert(key_id.into(), verify_key.key);
        }
    }

    keys
}

/// One key, from the database if it is held and from the network otherwise.
///
/// Which of the origin server and the notaries is asked first is the
/// operator's call: asking the origin first means a compromised notary is
/// only ever consulted for keys the origin could not answer for, while asking
/// the notaries first is faster, since one of them answers for many servers.
#[implement(super::Service)]
pub async fn get_verify_key(
    &self,
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> Result<VerifyKey> {
    let notary_first = self.services.server.config.query_trusted_key_servers_first;
    let notary_only = self.services.server.config.only_query_trusted_key_servers;

    if let Some(result) = self.verify_keys_for(origin).await.remove(key_id) {
        return Ok(result);
    }

    if notary_first && let Ok(result) = self.get_verify_key_from_notaries(origin, key_id).await {
        return Ok(result);
    }

    if !notary_only && let Ok(result) = self.get_verify_key_from_origin(origin, key_id).await {
        return Ok(result);
    }

    if !notary_first && let Ok(result) = self.get_verify_key_from_notaries(origin, key_id).await {
        return Ok(result);
    }

    Err!(BadServerResponse(debug_error!(
        "Failed to fetch federation signing-key {key_id:?} of {origin:?}"
    )))
}

#[implement(super::Service)]
async fn get_verify_key_from_notaries(
    &self,
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> Result<VerifyKey> {
    for notary in &self.services.server.config.trusted_servers {
        if let Ok(server_keys) = self.notary_request(notary, origin).await {
            // Everything the notary answered with is kept, not just the key
            // that was asked for: the rest is a fetch that will not have to
            // happen later.
            for server_key in server_keys.clone() {
                self.add_signing_keys(server_key).await?;
            }

            for server_key in server_keys {
                if let Some(result) = extract_key(server_key, key_id) {
                    return Ok(result);
                }
            }
        }
    }

    Err!(Request(NotFound(
        "Failed to fetch signing-key from notaries"
    )))
}

#[implement(super::Service)]
async fn get_verify_key_from_origin(
    &self,
    origin: &ServerName,
    key_id: &ServerSigningKeyId,
) -> Result<VerifyKey> {
    if let Ok(server_key) = self.server_request(origin).await {
        self.add_signing_keys(server_key.clone()).await?;
        if let Some(result) = extract_key(server_key, key_id) {
            return Ok(result);
        }
    }

    Err!(Request(NotFound("Failed to fetch signing-key from origin")))
}
