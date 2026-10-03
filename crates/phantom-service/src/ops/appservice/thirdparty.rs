use std::collections::BTreeMap;

use futures::StreamExt;
use phantom_core::{
    implement,
    stream::{IterStream, ReadyExt, WidebandExt},
};
use ruma::{
    api::appservice::{
        Registration,
        thirdparty::{get_location_for_protocol, get_protocol, get_user_for_protocol},
    },
    thirdparty::{Location, Protocol, User},
};

type Protocols = BTreeMap<String, Protocol>;

/// Fetches third-party protocol metadata from the registered appservices and
/// keys each response by protocol id. `only` restricts the fan-out to a single
/// protocol.
///
/// When several appservices advertise the same protocol, the first response
/// supplies the metadata and later responses add their `instances`.
/// Appservices without a usable destination and failed responses contribute
/// nothing rather than failing the client request.
#[implement(super::Service)]
#[tracing::instrument(level = "debug", skip(self))]
pub async fn thirdparty_protocols(&self, only: Option<&str>) -> Protocols {
    let jobs: Vec<(Registration, String)> = self
        .read()
        .await
        .values()
        .filter_map(|info| {
            info.registration
                .protocols
                .as_ref()
                .map(|protocols| (&info.registration, protocols))
        })
        .flat_map(|(registration, protocols)| {
            protocols
                .iter()
                .filter(|protocol| only.is_none_or(|only| only == protocol.as_str()))
                .map(move |protocol| (registration.clone(), protocol.clone()))
        })
        .collect();

    jobs.into_iter()
        .stream()
        .wide_filter_map(async |(registration, protocol)| {
            let request = get_protocol::v1::Request::new(protocol.clone());

            self.send_request(registration, request)
                .await
                .ok()
                .flatten()
                .map(|response| (protocol, response.protocol.into::<_>()))
        })
        .ready_fold(
            Protocols::new(),
            |mut protocols, (protocol, metadata): (String, Protocol)| {
                protocols
                    .entry(protocol)
                    .and_modify(|existing| existing.instances.extend(metadata.instances.clone()))
                    .or_insert(metadata);

                protocols
            },
        )
        .await
}

/// Looks up third-party users on `protocol` via the appservices declaring it,
/// forwarding `fields` to each and concatenating their results.
#[implement(super::Service)]
#[tracing::instrument(level = "debug", skip(self, fields))]
pub async fn thirdparty_users(
    &self,
    protocol: &str,
    fields: &BTreeMap<String, String>,
) -> Vec<User> {
    self.declaring(protocol)
        .await
        .into_iter()
        .stream()
        .wide_filter_map(async |registration| {
            let mut request = get_user_for_protocol::v1::Request::new(protocol.to_owned());
            request.fields = forwarded_fields(fields).collect();

            self.send_request(registration, request)
                .await
                .ok()
                .flatten()
        })
        .map(|response| response.users)
        .concat()
        .await
}

/// Looks up third-party locations on `protocol` via the appservices declaring
/// it, forwarding `fields` to each and concatenating their results.
#[implement(super::Service)]
#[tracing::instrument(level = "debug", skip(self, fields))]
pub async fn thirdparty_locations(
    &self,
    protocol: &str,
    fields: &BTreeMap<String, String>,
) -> Vec<Location> {
    self.declaring(protocol)
        .await
        .into_iter()
        .stream()
        .wide_filter_map(async |registration| {
            let mut request = get_location_for_protocol::v1::Request::new(protocol.to_owned());
            request.fields = forwarded_fields(fields).collect();

            self.send_request(registration, request)
                .await
                .ok()
                .flatten()
        })
        .map(|response| response.locations)
        .concat()
        .await
}

/// Snapshots the registrations declaring `protocol`. Cloning under the read
/// lock lets the fan-out release its guard before the first network await.
#[implement(super::Service)]
async fn declaring(&self, protocol: &str) -> Vec<Registration> {
    self.read()
        .await
        .values()
        .filter(|info| declares(&info.registration, protocol))
        .map(|info| info.registration.clone())
        .collect()
}

/// Drops the client's `access_token` from the forwarded query so a legacy
/// query-param credential never reaches the appservice.
fn forwarded_fields(
    fields: &BTreeMap<String, String>,
) -> impl Iterator<Item = (String, String)> + '_ {
    fields
        .iter()
        .filter(|(name, _)| name.as_str() != "access_token")
        .map(|(name, value)| (name.clone(), value.clone()))
}

fn declares(registration: &Registration, protocol: &str) -> bool {
    registration
        .protocols
        .as_ref()
        .is_some_and(|protocols| protocols.iter().any(|declared| declared == protocol))
}
