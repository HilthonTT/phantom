use std::collections::BTreeMap;

use axum::extract::State;
use futures::{StreamExt, stream::FuturesUnordered};
use phantom_core::Result;
use phantom_service::Services;
use ruma::{
    OwnedServerName, UserId,
    api::{client::keys::get_keys, federation::keys::get_keys as federation_get_keys},
};

use super::{Failures, execute_keys, record_failure};
use crate::{
    keys::{DeviceLists, LocalKeys, local_keys},
    router::Ruma,
};

/// # `POST /_matrix/client/r0/keys/query`
///
/// Get end-to-end encryption keys for the given users.
///
/// - Always fetches users from other servers over federation
/// - Gets master keys, self-signing keys, user signing keys and device keys.
/// - The master and self-signing keys contain signatures that the user is
///   allowed to see
pub(crate) async fn get_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_keys::v3::Request>,
) -> Result<get_keys::v3::Response> {
    let sender_user = body.sender_user();

    let (local, remote): (DeviceLists, DeviceLists) = body
        .device_keys
        .clone()
        .into_iter()
        .partition(|(user_id, _)| services.server_state.user_is_local(user_id));

    let allowed_signatures = |user_id: &UserId| user_id == sender_user;

    let mut keys = local_keys(
        &services,
        &local,
        Some(sender_user),
        &allowed_signatures,
        true,
    )
    .await;

    let failures = remote_keys(&services, remote, &mut keys).await;

    let mut response = get_keys::v3::Response::new();
    response.device_keys = keys.device_keys;
    response.master_keys = keys.master_keys;
    response.self_signing_keys = keys.self_signing_keys;
    response.user_signing_keys = keys.user_signing_keys;
    response.failures = failures;

    Ok(response)
}

/// Queries each remote server for its users' keys, merging what returns into
/// `keys`. Returns the servers which failed to answer.
async fn remote_keys(services: &Services, remote: DeviceLists, keys: &mut LocalKeys) -> Failures {
    let mut by_server: BTreeMap<OwnedServerName, DeviceLists> = BTreeMap::new();
    for (user_id, device_ids) in remote {
        by_server
            .entry(user_id.server_name().to_owned())
            .or_default()
            .insert(user_id, device_ids);
    }

    let mut pending: FuturesUnordered<_> = by_server
        .into_iter()
        .map(|(server, device_keys)| async move {
            let request = federation_get_keys::v1::Request::new(device_keys);
            let response = execute_keys(services, &server, request).await;

            (server, response)
        })
        .collect();

    let mut failures = Failures::new();
    while let Some((server, response)) = pending.next().await {
        match response {
            Ok(response) => {
                keys.device_keys.extend(response.device_keys);
                keys.master_keys.extend(response.master_keys);
                keys.self_signing_keys.extend(response.self_signing_keys);
            }
            Err(e) => record_failure(&mut failures, &server, &e),
        }
    }

    failures
}
