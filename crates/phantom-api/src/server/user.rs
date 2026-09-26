use axum::extract::State;
use futures::{FutureExt, StreamExt, TryFutureExt, future::join3};
use phantom_core::{Err, Result};
use ruma::{
    UInt, UserId,
    api::{
        client::device::Device,
        federation::{
            device::get_devices::{self, v1::UserDevice},
            keys::{claim_keys, get_keys},
        },
    },
};

use crate::{
    keys::{claim_local_one_time_keys, local_keys},
    router::Ruma,
};

pub(crate) async fn get_devices_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_devices::v1::Request>,
) -> Result<get_devices::v1::Response> {
    let user_id = &body.user_id;
    if !services.server_state.user_is_local(user_id) {
        return Err!(Request(InvalidParam(
            "Tried to access user from other server."
        )));
    }

    let allowed_signatures = |user: &UserId| user.server_name() == body.origin();
    let include_display_names = services
        .server
        .config
        .federation
        .allow_device_name_federation;

    let stream_id = services
        .users
        .get_devicelist_version(user_id)
        .map_ok(UInt::try_from)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();

    let master_key = services
        .users
        .get_master_key(None, user_id, &allowed_signatures)
        .map(Result::ok);

    let self_signing_key = services
        .users
        .get_self_signing_key(None, user_id, &allowed_signatures)
        .map(Result::ok);

    let devices = services
        .users
        .all_devices_metadata(user_id)
        .filter_map(
            async |Device {
                       device_id,
                       display_name,
                       ..
                   }: Device| {
                let keys = services
                    .users
                    .get_device_keys(user_id, &device_id)
                    .await
                    .ok()?;

                let display_name = include_display_names
                    .then_some(display_name)
                    .flatten()
                    .unwrap_or_else(|| device_id.to_string());

                let mut device = UserDevice::new(device_id, keys);
                device.device_display_name = Some(display_name);

                Some(device)
            },
        )
        .collect::<Vec<_>>();

    let (master_key, self_signing_key, devices) =
        join3(master_key, self_signing_key, devices).boxed().await;

    let mut response = get_devices::v1::Response::new(body.body.user_id, stream_id);
    response.devices = devices;
    response.master_key = master_key;
    response.self_signing_key = self_signing_key;

    Ok(response)
}

pub(crate) async fn get_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_keys::v1::Request>,
) -> Result<get_keys::v1::Response> {
    if body
        .device_keys
        .keys()
        .any(|user_id| !services.server_state.user_is_local(user_id))
    {
        return Err!(Request(InvalidParam(
            "User does not belong to this server."
        )));
    }

    let include_display_names = services
        .server
        .config
        .federation
        .allow_device_name_federation;

    let keys = local_keys(
        &services,
        &body.device_keys,
        None,
        &|user: &UserId| user.server_name() == body.origin(),
        include_display_names,
    )
    .await;

    let mut response = get_keys::v1::Response::new(keys.device_keys);
    response.master_keys = keys.master_keys;
    response.self_signing_keys = keys.self_signing_keys;

    Ok(response)
}

pub(crate) async fn claim_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<claim_keys::v1::Request>,
) -> Result<claim_keys::v1::Response> {
    if body
        .one_time_keys
        .keys()
        .any(|user_id| !services.server_state.user_is_local(user_id))
    {
        return Err!(Request(InvalidParam(
            "Tried to access user from other server."
        )));
    }

    let one_time_keys = claim_local_one_time_keys(&services, &body.one_time_keys).await;

    Ok(claim_keys::v1::Response::new(one_time_keys))
}
