use super::*;

impl Service {
    /// Adds a new device to a user.
    pub async fn create_device(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        token: &str,
        initial_device_display_name: Option<String>,
        client_ip: Option<String>,
    ) -> Result<()> {
        if !self.exists(user_id).await {
            return Err!(Request(InvalidParam(error!(
                "Called create_device for non-existent user {user_id}"
            ))));
        }

        let key = (user_id, device_id);
        let mut val = Device::new(device_id.into());
        val.display_name = initial_device_display_name;
        val.last_seen_ip = client_ip;
        val.last_seen_ts = Some(MilliSecondsSinceUnixEpoch::now());

        increment(&self.db.userid_devicelistversion, user_id.as_bytes());
        self.db.userdeviceid_metadata.put(key, Json(val)).ok();
        self.set_token(user_id, device_id, token).await
    }

    /// Removes a device from a user.
    pub async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) {
        let userdeviceid = (user_id, device_id);

        if let Ok(old_token) = self.db.userdeviceid_token.qry(&userdeviceid).await {
            self.db.userdeviceid_token.del(userdeviceid).ok();
            self.db.token_userdeviceid.remove(&old_token).ok();
        }

        let prefix = (user_id, device_id, Interfix);

        self.db.todeviceid_events.del_prefix(&prefix).await;

        // The device's one-time keys and identity keys go with it. Left behind,
        // a re-created device with the same id would advertise the old
        // session's identity keys, and peers claiming one of the stale
        // one-time keys would build Olm sessions the new client cannot open.
        self.db.onetimekeyid_onetimekeys.del_prefix(&prefix).await;

        self.db.keyid_key.del(userdeviceid).ok();

        increment(&self.db.userid_devicelistversion, user_id.as_bytes());

        self.db.userdeviceid_metadata.del(userdeviceid).ok();
        self.mark_device_key_update(user_id).await;
    }

    /// Returns an iterator over all device ids of this user.
    pub fn all_device_ids<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = &'a DeviceId> + Send + 'a {
        let prefix = (user_id, Interfix);
        self.db
            .userdeviceid_metadata
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, device_id): (Ignore, &str)| device_id.into())
    }

    pub async fn get_token(&self, user_id: &UserId, device_id: &DeviceId) -> Result<String> {
        let key = (user_id, device_id);
        self.db.userdeviceid_token.qry(&key).await.deserialized()
    }

    /// Replaces the access token of one device.
    pub async fn set_token(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        token: &str,
    ) -> Result<()> {
        let key = (user_id, device_id);
        if self.db.userdeviceid_metadata.qry(&key).await.is_err() {
            return Err!(Database(error!(
                ?user_id,
                ?device_id,
                message = "User does not exist or device has no metadata."
            )));
        }

        if let Ok(old_token) = self.db.userdeviceid_token.qry(&key).await {
            self.db.token_userdeviceid.remove(&old_token).ok();
        }

        self.db.userdeviceid_token.put_raw(key, token).ok();
        self.db.token_userdeviceid.raw_put(token, key).ok();

        Ok(())
    }

    pub async fn update_device_metadata(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        device: &Device,
    ) -> Result<()> {
        increment(&self.db.userid_devicelistversion, user_id.as_bytes());

        let key = (user_id, device_id);
        self.db.userdeviceid_metadata.put(key, Json(device)).ok();

        Ok(())
    }

    /// Get device metadata.
    pub async fn get_device_metadata(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Result<Device> {
        self.db
            .userdeviceid_metadata
            .qry(&(user_id, device_id))
            .await
            .deserialized()
    }

    pub async fn get_devicelist_version(&self, user_id: &UserId) -> Result<u64> {
        self.db
            .userid_devicelistversion
            .get(user_id)
            .await
            .deserialized()
    }

    pub fn all_devices_metadata<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = Device> + Send + 'a {
        let key = (user_id, Interfix);
        self.db
            .userdeviceid_metadata
            .stream_prefix(&key)
            .ignore_err()
            .map(|(_, val): (Ignore, Device)| val)
    }
}
