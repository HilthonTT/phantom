//! Fetching many missing keys at once.
//!
//! Verifying a batch of events one at a time would ask the network for one
//! key per event. This gathers what the whole batch needs first, drops what
//! is already held, and asks for the rest in as few requests as it can — one
//! per origin server in parallel, and one per notary for all of them at once.
//!
//! Nothing here returns the keys: they are written to the database as they
//! arrive, and the caller reads them back through
//! [`get`](super::Service::get_event_keys). What it does report, in the log,
//! is what could not be obtained.

use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use futures::{StreamExt, stream::FuturesUnordered};
use phantom_core::{
    debug, debug_error, debug_warn, error, implement, info, result::FlatOk, result::LogErr, trace,
    warn,
};
use ruma::{
    CanonicalJsonObject, OwnedServerName, OwnedServerSigningKeyId, ServerName, ServerSigningKeyId,
    api::federation::discovery::ServerSigningKeys, serde::Raw,
};
use serde_json::value::RawValue as RawJsonValue;
use tokio::time::{Instant, timeout_at};

use super::key_exists;

/// A server request is given this long to answer before the keys it was asked
/// for are counted as missing. It bounds the whole round of origin requests,
/// not each one: they run together.
const ORIGIN_TIMEOUT: Duration = Duration::from_secs(45);

/// What is still missing, by server.
type Batch = BTreeMap<OwnedServerName, Vec<OwnedServerSigningKeyId>>;

/// Acquires every key needed to verify `events`.
#[implement(super::Service)]
pub async fn acquire_events_pubkeys<'a, I>(&self, events: I)
where
    I: Iterator<Item = &'a Box<RawJsonValue>> + Send,
{
    type Batch = BTreeMap<OwnedServerName, BTreeSet<OwnedServerSigningKeyId>>;
    type Signatures = BTreeMap<OwnedServerName, BTreeMap<OwnedServerSigningKeyId, String>>;

    let mut batch = Batch::new();
    events
        .cloned()
        .map(Raw::<CanonicalJsonObject>::from_json)
        .map(|event| event.get_field::<Signatures>("signatures"))
        .filter_map(FlatOk::flat_ok)
        .flat_map(IntoIterator::into_iter)
        .for_each(|(server, sigs)| {
            batch.entry(server).or_default().extend(sigs.into_keys());
        });

    let batch = batch
        .iter()
        .map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

    self.acquire_pubkeys(batch).await;
}

#[implement(super::Service)]
pub async fn acquire_pubkeys<'a, S, K>(&self, batch: S)
where
    S: Iterator<Item = (&'a ServerName, K)> + Send + Clone,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send + Clone,
{
    let config = &self.services.server.config;
    let notary_only = config.only_query_trusted_key_servers;
    let notary_first_always = config.query_trusted_key_servers_first;
    let notary_first_on_join = config.query_trusted_key_servers_first_on_join;

    let requested_servers = batch.clone().count();
    let requested_keys = batch.clone().flat_map(|(_, key_ids)| key_ids).count();

    debug!("acquire {requested_keys} keys from {requested_servers}");

    let mut missing = self.acquire_locals(batch).await;
    let mut missing_keys = keys_count(&missing);
    let mut missing_servers = missing.len();
    if missing_servers == 0 {
        return;
    }

    info!("{missing_keys} keys for {missing_servers} servers will be acquired");

    if notary_first_always || notary_first_on_join {
        missing = self.acquire_notary(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        warn!("missing {missing_keys} keys for {missing_servers} servers from all notaries first");
    }

    if !notary_only {
        missing = self.acquire_origins(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        debug_warn!("missing {missing_keys} keys for {missing_servers} servers unreachable");
    }

    if !notary_first_always && !notary_first_on_join {
        missing = self.acquire_notary(missing.into_iter()).await;
        missing_keys = keys_count(&missing);
        missing_servers = missing.len();
        if missing_keys == 0 {
            return;
        }

        debug_warn!(
            "still missing {missing_keys} keys for {missing_servers} servers from all notaries."
        );
    }

    if missing_keys > 0 {
        warn!(
            "did not obtain {missing_keys} keys for {missing_servers} servers out of \
             {requested_keys} total keys for {requested_servers} total servers."
        );
    }

    for (server, key_ids) in missing {
        debug_warn!(?server, ?key_ids, "missing");
    }
}

/// Whatever of `batch` is not already in the database.
#[implement(super::Service)]
async fn acquire_locals<'a, S, K>(&self, batch: S) -> Batch
where
    S: Iterator<Item = (&'a ServerName, K)> + Send,
    K: Iterator<Item = &'a ServerSigningKeyId> + Send,
{
    let mut missing = Batch::new();
    for (server, key_ids) in batch {
        for key_id in key_ids {
            if !self.verify_key_exists(server, key_id).await {
                missing
                    .entry(server.into())
                    .or_default()
                    .push(key_id.into());
            }
        }
    }

    missing
}

/// Asks every server in `batch` for its own keys, all at once.
#[implement(super::Service)]
async fn acquire_origins<I>(&self, batch: I) -> Batch
where
    I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
    let timeout = Instant::now()
        .checked_add(ORIGIN_TIMEOUT)
        .expect("timeout overflows");

    let mut requests: FuturesUnordered<_> = batch
        .map(|(origin, key_ids)| self.acquire_origin(origin, key_ids, timeout))
        .collect();

    let mut missing = Batch::new();
    while let Some((origin, key_ids)) = requests.next().await {
        if !key_ids.is_empty() {
            missing.insert(origin, key_ids);
        }
    }

    missing
}

#[implement(super::Service)]
async fn acquire_origin(
    &self,
    origin: OwnedServerName,
    mut key_ids: Vec<OwnedServerSigningKeyId>,
    timeout: Instant,
) -> (OwnedServerName, Vec<OwnedServerSigningKeyId>) {
    match timeout_at(timeout, self.server_request(&origin)).await {
        Err(e) => debug_warn!(?origin, "timed out: {e}"),
        Ok(Err(e)) => debug_error!(?origin, "{e}"),
        Ok(Ok(server_keys)) => {
            trace!(%origin, ?key_ids, ?server_keys, "received server_keys");

            if self
                .add_signing_keys(server_keys.clone())
                .await
                .log_err()
                .is_ok()
            {
                key_ids.retain(|key_id| !key_exists(&server_keys, key_id));
            }
        }
    }

    (origin, key_ids)
}

/// Asks each notary in turn about everything still missing, so that each one
/// is asked for less than the last and none is asked at all once nothing is
/// left to find.
#[implement(super::Service)]
async fn acquire_notary<I>(&self, batch: I) -> Batch
where
    I: Iterator<Item = (OwnedServerName, Vec<OwnedServerSigningKeyId>)> + Send,
{
    let mut missing: Batch = batch.collect();
    for notary in &self.services.server.config.trusted_servers {
        if missing.is_empty() {
            break;
        }

        let missing_keys = keys_count(&missing);
        let missing_servers = missing.len();
        debug!(
            "Asking notary {notary} for {missing_keys} missing keys from {missing_servers} servers"
        );

        let batch = missing
            .iter()
            .map(|(server, keys)| (server.borrow(), keys.iter().map(Borrow::borrow)));

        match self.batch_notary_request(notary, batch).await {
            Err(e) => error!("Failed to contact notary {notary:?}: {e}"),
            Ok(results) => {
                for server_keys in results {
                    self.acquire_notary_result(&mut missing, server_keys).await;
                }
            }
        }
    }

    missing
}

#[implement(super::Service)]
async fn acquire_notary_result(&self, missing: &mut Batch, server_keys: ServerSigningKeys) {
    let server = &server_keys.server_name;
    if self
        .add_signing_keys(server_keys.clone())
        .await
        .log_err()
        .is_err()
    {
        return;
    }

    if let Some(key_ids) = missing.get_mut(server) {
        key_ids.retain(|key_id| !key_exists(&server_keys, key_id));
        if key_ids.is_empty() {
            missing.remove(server);
        }
    }
}

fn keys_count(batch: &Batch) -> usize {
    batch.values().flat_map(|key_ids| key_ids.iter()).count()
}
