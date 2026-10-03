use std::collections::BTreeMap;

use axum::extract::State;
use futures::{StreamExt, stream::FuturesUnordered};
use phantom_core::Result;
use ruma::{
    OwnedServerName,
    api::{client::keys::claim_keys, federation::keys::claim_keys as federation_claim_keys},
};

use super::{Failures, execute_keys, record_failure};
use crate::{
    keys::{OneTimeKeyClaims, claim_local_one_time_keys},
    router::Ruma,
};

/// # `POST /_matrix/client/r0/keys/claim`
///
/// Claims one-time keys
pub(crate) async fn claim_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<claim_keys::v3::Request>,
) -> Result<claim_keys::v3::Response> {
    let (local, remote): (OneTimeKeyClaims, OneTimeKeyClaims) = body
        .one_time_keys
        .clone()
        .into_iter()
        .partition(|(user_id, _)| services.server_state.user_is_local(user_id));

    let mut one_time_keys = claim_local_one_time_keys(&services, &local).await;

    let mut by_server: BTreeMap<OwnedServerName, OneTimeKeyClaims> = BTreeMap::new();
    for (user_id, claims) in remote {
        by_server
            .entry(user_id.server_name().to_owned())
            .or_default()
            .insert(user_id, claims);
    }

    let services = &*services;
    let mut pending: FuturesUnordered<_> = by_server
        .into_iter()
        .map(|(server, claims)| async move {
            let request = federation_claim_keys::v1::Request::new(claims);
            let response = execute_keys(services, &server, request).await;

            (server, response)
        })
        .collect();

    let mut failures = Failures::new();
    while let Some((server, response)) = pending.next().await {
        match response {
            Ok(response) => one_time_keys.extend(response.one_time_keys),
            Err(e) => record_failure(&mut failures, &server, &e),
        }
    }

    let mut response = claim_keys::v3::Response::new(one_time_keys);
    response.failures = failures;

    Ok(response)
}
