use std::collections::BTreeMap;

use futures::{FutureExt, StreamExt, TryFutureExt, future::join4, stream::FuturesUnordered};
use phantom_core::{Result, debug_warn};
use phantom_service::Services;
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, DeviceId, OneTimeKeyAlgorithm, OwnedDeviceId,
    OwnedOneTimeKeyId, OwnedUserId, UserId,
    api::client::device::Device,
    encryption::{CrossSigningKey, DeviceKeys, OneTimeKey},
    serde::Raw,
};
use serde_json::value::to_raw_value;

pub(crate) type DeviceLists = BTreeMap<OwnedUserId, Vec<OwnedDeviceId>>;
pub(crate) type DeviceKeyMap = BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, Raw<DeviceKeys>>>;
pub(crate) type CrossSigningKeys = BTreeMap<OwnedUserId, Raw<CrossSigningKey>>;
pub(crate) type OneTimeKeyClaims =
    BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, OneTimeKeyAlgorithm>>;
pub(crate) type OneTimeKeyMap =
    BTreeMap<OwnedUserId, BTreeMap<OwnedDeviceId, BTreeMap<OwnedOneTimeKeyId, Raw<OneTimeKey>>>>;

#[derive(Default)]
pub(crate) struct LocalKeys {
    pub(crate) device_keys: DeviceKeyMap,
    pub(crate) master_keys: CrossSigningKeys,
    pub(crate) self_signing_keys: CrossSigningKeys,
    pub(crate) user_signing_keys: CrossSigningKeys,
}

impl LocalKeys {
    fn merge(mut self, other: Self) -> Self {
        self.device_keys.extend(other.device_keys);
        self.master_keys.extend(other.master_keys);
        self.self_signing_keys.extend(other.self_signing_keys);
        self.user_signing_keys.extend(other.user_signing_keys);
        self
    }
}

pub(crate) async fn local_keys<F>(
    services: &Services,
    users: &DeviceLists,
    sender_user: Option<&UserId>,
    allowed_signatures: &F,
    include_display_names: bool,
) -> LocalKeys
where
    F: Fn(&UserId) -> bool + Send + Sync,
{
    let mut pending = FuturesUnordered::new();
    for (user_id, device_ids) in users {
        pending.push(user_keys(
            services,
            user_id,
            device_ids,
            sender_user,
            allowed_signatures,
            include_display_names,
        ));
    }

    let mut keys = LocalKeys::default();
    while let Some(user_keys) = pending.next().await {
        keys = keys.merge(user_keys);
    }

    keys
}

async fn user_keys<F>(
    services: &Services,
    user_id: &UserId,
    device_ids: &[OwnedDeviceId],
    sender_user: Option<&UserId>,
    allowed_signatures: &F,
    include_display_names: bool,
) -> LocalKeys
where
    F: Fn(&UserId) -> bool + Send + Sync,
{
    let device_keys = device_keys(services, user_id, device_ids, include_display_names);

    let master_key = services
        .users
        .get_master_key(sender_user, user_id, allowed_signatures)
        .map(Result::ok);

    let self_signing_key = services
        .users
        .get_self_signing_key(sender_user, user_id, allowed_signatures)
        .map(Result::ok);

    let user_signing_key = async {
        if sender_user == Some(user_id) {
            services.users.get_user_signing_key(user_id).await.ok()
        } else {
            None
        }
    };

    let (device_keys, master_key, self_signing_key, user_signing_key) =
        join4(device_keys, master_key, self_signing_key, user_signing_key).await;

    let keyed = |key: Option<Raw<CrossSigningKey>>| {
        key.map(|key| (user_id.to_owned(), key))
            .into_iter()
            .collect()
    };

    LocalKeys {
        device_keys: BTreeMap::from([(user_id.to_owned(), device_keys)]),
        master_keys: keyed(master_key),
        self_signing_keys: keyed(self_signing_key),
        user_signing_keys: keyed(user_signing_key),
    }
}

async fn device_keys(
    services: &Services,
    user_id: &UserId,
    device_ids: &[OwnedDeviceId],
    include_display_names: bool,
) -> BTreeMap<OwnedDeviceId, Raw<DeviceKeys>> {
    let device_ids = if device_ids.is_empty() {
        services
            .users
            .all_device_ids(user_id)
            .map(ToOwned::to_owned)
            .collect()
            .await
    } else {
        device_ids.to_vec()
    };

    let mut pending = FuturesUnordered::new();
    for device_id in device_ids {
        pending.push(async move {
            let keys =
                device_keys_with_display_name(services, user_id, &device_id, include_display_names)
                    .await;

            keys.map(|keys| (device_id, keys))
        });
    }

    let mut keys = BTreeMap::new();
    while let Some(device_keys) = pending.next().await {
        keys.extend(device_keys);
    }

    keys
}

async fn device_keys_with_display_name(
    services: &Services,
    user_id: &UserId,
    device_id: &DeviceId,
    include_display_names: bool,
) -> Option<Raw<DeviceKeys>> {
    let keys = services
        .users
        .get_device_keys(user_id, device_id)
        .await
        .ok()?;

    let metadata = services
        .users
        .get_device_metadata(user_id, device_id)
        .inspect_err(|e| debug_warn!("Device metadata missing for {user_id} {device_id}: {e}"))
        .await
        .ok()?;

    with_display_name(keys, metadata, include_display_names)
        .inspect_err(|e| debug_warn!("Invalid device keys for {user_id} {device_id}: {e}"))
        .ok()
}

fn with_display_name(
    keys: Raw<DeviceKeys>,
    metadata: Device,
    include_display_names: bool,
) -> Result<Raw<DeviceKeys>> {
    let Some(display_name) = metadata.display_name else {
        return Ok(keys);
    };

    let display_name = if include_display_names {
        display_name
    } else {
        metadata.device_id.to_string()
    };

    let mut object = keys.deserialize_as_unchecked::<CanonicalJsonObject>()?;

    if let CanonicalJsonValue::Object(unsigned) = object
        .entry("unsigned".into())
        .or_insert_with(|| CanonicalJsonObject::new().into())
    {
        unsigned.insert(
            "device_display_name".into(),
            CanonicalJsonValue::String(display_name),
        );
    }

    Ok(Raw::from_json(to_raw_value(&object)?))
}

pub(crate) async fn claim_local_one_time_keys(
    services: &Services,
    claims: &OneTimeKeyClaims,
) -> OneTimeKeyMap {
    let mut claimed = OneTimeKeyMap::new();
    for (user_id, requested) in claims {
        let mut device_keys = BTreeMap::new();
        for (device_id, algorithm) in requested {
            if let Ok((key_id, key)) = services
                .users
                .take_one_time_key(user_id, device_id, algorithm)
                .await
            {
                device_keys.insert(device_id.clone(), BTreeMap::from([(key_id, key)]));
            }
        }

        if !device_keys.is_empty() {
            claimed.insert(user_id.clone(), device_keys);
        }
    }

    claimed
}
