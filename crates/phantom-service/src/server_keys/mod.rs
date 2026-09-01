//! The signing keys this server has, and the ones it has learned about.
//!
//! Every event and every federation request carries a signature, so verifying
//! either means holding the public key the far end signed with. This service
//! owns both halves of that: [`keypair`] is this server's own key, loaded or
//! generated once at startup, and the `server_signingkeys` column is what it
//! has learned about everyone else's.
//!
//! A key is not simply looked up. [`acquire`] fetches what is missing, from
//! the origin server or from a notary, and [`get`] assembles the key map a
//! single verification needs; [`request`] is the federation requests those
//! two make. [`sign`] and [`verify`] are the two uses all of it exists for.
//!
//! Keys are never evicted. A server rotating its key publishes the old one as
//! an `old_verify_key`, and an event signed years ago is still verified
//! against the key that signed it, so what is learned here is kept.

mod acquire;
mod get;
mod keypair;
mod request;
mod sign;
mod verify;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::StreamExt;
use phantom_core::{
    Err, Result, implement, server::Server, stream::IterStream, time::timepoint_from_now,
};
use phantom_database::{Deserialized, Json, Map};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, MilliSecondsSinceUnixEpoch, OwnedServerName,
    OwnedServerSigningKeyId, RoomVersionId, ServerName, ServerSigningKeyId,
    api::federation::discovery::{ServerSigningKeys, VerifyKey},
    room_version_rules::SignaturesRules,
    serde::Raw,
    signatures::{Ed25519KeyPair, PublicKeyMap, PublicKeySet},
};
use serde_json::value::RawValue as RawJsonValue;

use crate::{Dep, federation, server_state};

pub struct Service {
    keypair: Box<Ed25519KeyPair>,
    verify_keys: VerifyKeys,
    minimum_valid: Duration,
    services: Services,
    db: Data,
}

struct Services {
    server_state: Dep<server_state::Service>,
    federation: Dep<federation::Service>,
    server: Arc<Server>,
}

struct Data {
    server_signingkeys: Arc<Map>,
}

pub type VerifyKeys = BTreeMap<OwnedServerSigningKeyId, VerifyKey>;
pub type PubKeyMap = PublicKeyMap;
pub type PubKeys = PublicKeySet;

/// Which servers had to be asked for which of their keys.
type RequiredKeys = BTreeMap<OwnedServerName, Vec<OwnedServerSigningKeyId>>;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        let minimum_valid = Duration::from_secs(3600);

        let (keypair, verify_keys) = keypair::init(args.db)?;
        debug_assert!(
            verify_keys.len() == 1,
            "only one active verify_key supported"
        );

        Ok(Arc::new(Self {
            keypair,
            verify_keys,
            minimum_valid,
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
                federation: args.depend::<federation::Service>("federation"),
                server: args.server.clone(),
            },
            db: Data {
                server_signingkeys: args.db["server_signingkeys"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
#[inline]
pub fn keypair(&self) -> &Ed25519KeyPair {
    &self.keypair
}

#[implement(Service)]
#[inline]
pub fn active_key_id(&self) -> &ServerSigningKeyId {
    self.active_verify_key().0
}

#[implement(Service)]
#[inline]
pub fn active_verify_key(&self) -> (&ServerSigningKeyId, &VerifyKey) {
    debug_assert!(
        self.verify_keys.len() <= 1,
        "more than one active verify_key"
    );
    self.verify_keys
        .iter()
        .next()
        .map(|(id, key)| (id.as_ref(), key))
        .expect("missing active verify_key")
}

/// Merges what was just learned about `new_keys.server_name` into what is
/// already held for it.
///
/// Not atomic — a concurrent call can read the same starting point and write
/// over this one — but the two writers are merging into the same set of keys
/// rather than replacing it, and a key lost that way is re-fetched the next
/// time it is needed.
#[implement(Service)]
async fn add_signing_keys(&self, new_keys: ServerSigningKeys) -> Result {
    let origin = &new_keys.server_name;

    let mut keys: ServerSigningKeys = self
        .db
        .server_signingkeys
        .get(origin)
        .await
        .deserialized()
        .unwrap_or_else(|_| {
            ServerSigningKeys::new(origin.to_owned(), MilliSecondsSinceUnixEpoch::now())
        });

    keys.verify_keys.extend(new_keys.verify_keys);
    keys.old_verify_keys.extend(new_keys.old_verify_keys);

    self.db.server_signingkeys.raw_put(origin, Json(&keys))
}

/// Whether every key needed to verify `object` is already held, so that
/// verifying it will not have to go out to the network.
#[implement(Service)]
pub async fn required_keys_exist(
    &self,
    object: &CanonicalJsonObject,
    version: &RoomVersionId,
) -> bool {
    let Some(rules) = version.rules() else {
        return false;
    };

    let Ok(required) = required_keys(object, &rules.signatures) else {
        return false;
    };

    required
        .iter()
        .flat_map(|(server, key_ids)| key_ids.iter().map(move |key_id| (server, key_id)))
        .stream()
        .all(|(server, key_id)| self.verify_key_exists(server, key_id))
        .await
}

#[implement(Service)]
pub async fn verify_key_exists(&self, origin: &ServerName, key_id: &ServerSigningKeyId) -> bool {
    type KeysMap<'a> = BTreeMap<OwnedServerSigningKeyId, &'a RawJsonValue>;

    let Ok(keys) = self
        .db
        .server_signingkeys
        .get(origin)
        .await
        .deserialized::<Raw<ServerSigningKeys>>()
    else {
        return false;
    };

    if let Ok(Some(verify_keys)) = keys.get_field::<KeysMap<'_>>("verify_keys")
        && verify_keys.contains_key(key_id)
    {
        return true;
    }

    if let Ok(Some(old_verify_keys)) = keys.get_field::<KeysMap<'_>>("old_verify_keys")
        && old_verify_keys.contains_key(key_id)
    {
        return true;
    }

    false
}

#[implement(Service)]
pub async fn verify_keys_for(&self, origin: &ServerName) -> VerifyKeys {
    let mut keys = self
        .signing_keys_for(origin)
        .await
        .map(|keys| merge_old_keys(keys).verify_keys)
        .unwrap_or_default();

    if self.services.server_state.server_is_ours(origin) {
        keys.extend(self.verify_keys.clone());
    }

    keys
}

#[implement(Service)]
pub async fn signing_keys_for(&self, origin: &ServerName) -> Result<ServerSigningKeys> {
    self.db.server_signingkeys.get(origin).await.deserialized()
}

/// The oldest `valid_until_ts` this server will accept in an answer from a
/// notary, so that a notary cannot satisfy a query with a cached key that has
/// long since expired.
#[implement(Service)]
fn minimum_valid_ts(&self) -> MilliSecondsSinceUnixEpoch {
    let timepoint = timepoint_from_now(self.minimum_valid).expect("SystemTime should not overflow");
    MilliSecondsSinceUnixEpoch::from_system_time(timepoint).expect("UInt should not overflow")
}

/// Which key of which server has to be checked to verify `object`.
///
/// ruma names the servers whose signature must be checked, but not which key
/// each of them signed with; that half is only in the event's own
/// `signatures`. This puts the two back together.
fn required_keys(object: &CanonicalJsonObject, rules: &SignaturesRules) -> Result<RequiredKeys> {
    use ruma::signatures::required_server_signatures_to_verify_event;

    let servers = required_server_signatures_to_verify_event(object, rules)?;

    let Some(CanonicalJsonValue::Object(signatures)) = object.get("signatures") else {
        return Err!(BadServerResponse("Event has no signatures."));
    };

    servers
        .into_iter()
        .map(|server| {
            let Some(CanonicalJsonValue::Object(signed)) = signatures.get(server.as_str()) else {
                return Err!(BadServerResponse("Event is not signed by {server}."));
            };

            let key_ids: Vec<OwnedServerSigningKeyId> = signed
                .keys()
                .map(|key_id| key_id.as_str().try_into())
                .collect::<Result<_, _>>()?;

            Ok((server, key_ids))
        })
        .collect()
}

/// The keys a server published, with the ones it has since rotated away from
/// folded in.
///
/// An event is verified against the key that signed it, which for an old
/// event is a key the server has already replaced. Nothing distinguishes the
/// two at the point of verification, so they are looked up as one set.
fn merge_old_keys(mut keys: ServerSigningKeys) -> ServerSigningKeys {
    keys.verify_keys.extend(
        keys.old_verify_keys
            .clone()
            .into_iter()
            .map(|(key_id, old)| (key_id, VerifyKey::new(old.key))),
    );

    keys
}

fn extract_key(mut keys: ServerSigningKeys, key_id: &ServerSigningKeyId) -> Option<VerifyKey> {
    keys.verify_keys.remove(key_id).or_else(|| {
        keys.old_verify_keys
            .remove(key_id)
            .map(|old| VerifyKey::new(old.key))
    })
}

fn key_exists(keys: &ServerSigningKeys, key_id: &ServerSigningKeyId) -> bool {
    keys.verify_keys.contains_key(key_id) || keys.old_verify_keys.contains_key(key_id)
}
