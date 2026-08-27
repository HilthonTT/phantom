//! The resolved-destination cache, which lives in the database rather than in
//! memory.
//!
//! Two columns. `servername_destination` holds what a server name resolved
//! to, the answer to the whole spec procedure. `servername_override` holds
//! the addresses a hostname resolved to, which is what [`super::dns`] answers
//! reqwest from so that the connection is opened to the address the
//! resolution decided on rather than to whatever DNS says at connect time.
//!
//! Both carry their own expiry rather than relying on the column being
//! cleared, and both expire at a randomized time so that a server which
//! learned about many destinations at once does not re-resolve them all at
//! once either.

use std::{net::IpAddr, sync::Arc, time::SystemTime};

use arrayvec::ArrayVec;
use futures::{Stream, StreamExt, future::join};
use phantom_core::{Result, at, err, implement, math::Expected, rand, stream::TryIgnore};
use phantom_database::{Cbor, Deserialized, Map};
use ruma::ServerName;
use serde::{Deserialize, Serialize};

use super::destination::Destination;

pub struct Cache {
    destinations: Arc<Map>,
    overrides: Arc<Map>,
}

/// What a server name resolved to, and the `Host` header that goes with it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CachedDest {
    pub dest: Destination,
    pub host: String,
    pub expire: SystemTime,
}

/// The addresses a hostname resolved to, and the port they are reached on.
///
/// `overriding` is set where the name this is stored under is not the name
/// that was resolved — an SRV record pointing elsewhere — which is what lets
/// [`super::dns`] follow the indirection a second time rather than treating
/// the cached addresses as final.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CachedOverride {
    pub ips: IpAddrs,
    pub port: u16,
    pub expire: SystemTime,
    pub overriding: Option<String>,
}

pub type IpAddrs = ArrayVec<IpAddr, MAX_IPS>;

/// Addresses kept per name. A server publishing more than a handful is
/// load-balancing, and trying all of them is not this cache's job.
pub(crate) const MAX_IPS: usize = 3;

impl Cache {
    pub(super) fn new(args: &crate::Args<'_>) -> Arc<Self> {
        Arc::new(Self {
            destinations: args.db["servername_destination"].clone(),
            overrides: args.db["servername_override"].clone(),
        })
    }
}

#[implement(Cache)]
pub async fn clear(&self) {
    join(self.clear_destinations(), self.clear_overrides()).await;
}

#[implement(Cache)]
pub async fn clear_destinations(&self) {
    self.destinations.clear().await;
}

#[implement(Cache)]
pub async fn clear_overrides(&self) {
    self.overrides.clear().await;
}

#[implement(Cache)]
pub fn del_destination(&self, name: &ServerName) -> Result {
    self.destinations.remove(name)
}

#[implement(Cache)]
pub fn del_override(&self, name: &str) -> Result {
    self.overrides.remove(name)
}

#[implement(Cache)]
pub fn set_destination(&self, name: &ServerName, dest: &CachedDest) -> Result {
    self.destinations.raw_put(name, Cbor(dest))
}

#[implement(Cache)]
pub fn set_override(&self, name: &str, over: &CachedOverride) -> Result {
    self.overrides.raw_put(name, Cbor(over))
}

#[implement(Cache)]
#[must_use]
pub async fn has_destination(&self, destination: &ServerName) -> bool {
    self.get_destination(destination).await.is_ok()
}

#[implement(Cache)]
#[must_use]
pub async fn has_override(&self, destination: &str) -> bool {
    self.get_override(destination)
        .await
        .as_ref()
        .is_ok_and(CachedOverride::valid)
}

#[implement(Cache)]
pub async fn get_destination(&self, name: &ServerName) -> Result<CachedDest> {
    self.destinations
        .get(name)
        .await
        .deserialized::<Cbor<_>>()
        .map(at!(0))
        .into_iter()
        .find(CachedDest::valid)
        .ok_or_else(|| err!(Request(NotFound("Expired from cache"))))
}

#[implement(Cache)]
pub async fn get_override(&self, name: &str) -> Result<CachedOverride> {
    self.overrides
        .get(name)
        .await
        .deserialized::<Cbor<_>>()
        .map(at!(0))
}

/// Every cached destination, expired ones included.
///
/// The name comes back as `&str` rather than `&ServerName`: what is in the
/// column is whatever was resolved, and a borrowed `ServerName` cannot be
/// deserialized without asserting that it is still valid.
#[implement(Cache)]
pub fn destinations(&self) -> impl Stream<Item = (&str, CachedDest)> + Send + '_ {
    self.destinations
        .stream()
        .ignore_err()
        .map(|item: (&str, Cbor<_>)| (item.0, item.1.0))
}

/// Every cached address override, expired ones included.
#[implement(Cache)]
pub fn overrides(&self) -> impl Stream<Item = (&str, CachedOverride)> + Send + '_ {
    self.overrides
        .stream()
        .ignore_err()
        .map(|item: (&str, Cbor<_>)| (item.0, item.1.0))
}

impl CachedDest {
    #[inline]
    #[must_use]
    pub fn valid(&self) -> bool {
        self.expire > SystemTime::now()
    }

    /// Between 18 and 36 hours out. A resolved destination changes rarely,
    /// and the spread is what keeps every destination learned during one
    /// startup from expiring together.
    #[must_use]
    pub(crate) fn default_expire() -> SystemTime {
        rand::time_from_now_secs(60 * 60 * 18..60 * 60 * 36)
    }

    #[inline]
    #[must_use]
    pub fn size(&self) -> usize {
        self.dest
            .size()
            .expected_add(self.host.len())
            .expected_add(size_of_val(&self.expire))
    }
}

impl CachedOverride {
    #[inline]
    #[must_use]
    pub fn valid(&self) -> bool {
        self.expire > SystemTime::now()
    }

    /// Between 6 and 12 hours out — shorter than a destination's, since this
    /// is an address rather than the decision about which name to resolve.
    #[must_use]
    pub(crate) fn default_expire() -> SystemTime {
        rand::time_from_now_secs(60 * 60 * 6..60 * 60 * 12)
    }

    #[inline]
    #[must_use]
    pub fn size(&self) -> usize {
        size_of_val(self)
    }
}
