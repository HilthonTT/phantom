use ruma::OwnedOneTimeKeyId;

use super::*;

/// A device keeps one fallback key per algorithm. Claiming it marks it used
/// rather than removing it, so it keeps being handed out until the device
/// uploads a replacement.
#[derive(Deserialize, Serialize)]
struct FallbackKey {
    key_id: OwnedOneTimeKeyId,
    key: Raw<OneTimeKey>,
    used: bool,
}

impl Service {
    pub async fn set_fallback_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        key_id: &OneTimeKeyId,
        key: &Raw<OneTimeKey>,
    ) -> Result {
        if self
            .db
            .userdeviceid_metadata
            .qry(&(user_id, device_id))
            .await
            .is_err()
        {
            return Err!(Database(error!(
                ?user_id,
                ?device_id,
                message = "User does not exist or device has no metadata."
            )));
        }

        let _lock = self.one_time_key_locks.lock(user_id).await;

        let algorithm = key_id.algorithm();
        let db_key = (user_id, device_id, algorithm.as_str());

        // Clients may upload the same fallback key again; that must not make a
        // used key unused.
        let unchanged = self
            .db
            .fallbackkeyid_fallbackkey
            .qry(&db_key)
            .await
            .deserialized::<FallbackKey>()
            .is_ok_and(|stored| {
                stored.key_id == key_id && stored.key.json().get() == key.json().get()
            });

        if unchanged {
            return Ok(());
        }

        let value = FallbackKey {
            key_id: key_id.to_owned(),
            key: key.clone(),
            used: false,
        };

        self.db
            .fallbackkeyid_fallbackkey
            .put(db_key, Json(value))
            .ok();

        self.mark_one_time_key_update(user_id)
    }

    pub async fn take_fallback_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        algorithm: &OneTimeKeyAlgorithm,
    ) -> Result<(OwnedOneTimeKeyId, Raw<OneTimeKey>)> {
        let _lock = self.one_time_key_locks.lock(user_id).await;

        let db_key = (user_id, device_id, algorithm.as_str());
        let mut stored: FallbackKey = self
            .db
            .fallbackkeyid_fallbackkey
            .qry(&db_key)
            .await
            .deserialized()
            .map_err(|_| err!(Request(NotFound("No fallback key found"))))?;

        if !stored.used {
            stored.used = true;
            self.db
                .fallbackkeyid_fallbackkey
                .put(db_key, Json(&stored))
                .ok();

            self.mark_one_time_key_update(user_id)?;
        }

        Ok((stored.key_id, stored.key))
    }

    pub async fn unused_fallback_key_types(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Vec<OneTimeKeyAlgorithm> {
        type KeyVal = ((Ignore, Ignore, Ignore), FallbackKey);

        let query = (user_id, device_id, Interfix);
        self.db
            .fallbackkeyid_fallbackkey
            .stream_prefix(&query)
            .ignore_err()
            .ready_filter_map(|((Ignore, Ignore, Ignore), stored): KeyVal| {
                (!stored.used).then(|| stored.key_id.algorithm())
            })
            .collect()
            .await
    }

    fn mark_one_time_key_update(&self, user_id: &UserId) -> Result {
        let count = self.services.server_state.next_count()?;
        self.db
            .userid_lastonetimekeyupdate
            .raw_put(user_id, count)
            .ok();

        Ok(())
    }
}
