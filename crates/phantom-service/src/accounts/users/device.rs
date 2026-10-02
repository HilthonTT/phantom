use super::*;

impl Service {
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

    pub async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) {
        let userdeviceid = (user_id, device_id);

        self.remove_tokens(user_id, device_id).await;

        let prefix = (user_id, device_id, Interfix);

        self.db.todeviceid_events.del_prefix(&prefix).await;

        self.db.onetimekeyid_onetimekeys.del_prefix(&prefix).await;

        self.db.keyid_key.del(userdeviceid).ok();

        // Pushers registered from this device must stop with it (this also runs
        // for every device on deactivation).
        let pushkeys: Vec<_> = self
            .services
            .pusher
            .get_pushkeys(user_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        for pushkey in pushkeys {
            if self
                .services
                .pusher
                .get_pusher_device(&pushkey)
                .await
                .is_ok_and(|pusher_device| pusher_device == device_id)
            {
                self.services.pusher.delete_pusher(user_id, &pushkey).ok();
            }
        }

        increment(&self.db.userid_devicelistversion, user_id.as_bytes());

        self.db.userdeviceid_metadata.del(userdeviceid).ok();
        self.db.oidcdevice_userdeviceid.del(userdeviceid).ok();
        self.mark_device_key_update(user_id).await;
    }

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

    /// Replace the access token of one device with a non-expiring one.
    pub async fn set_token(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        token: &str,
    ) -> Result<()> {
        self.set_access_token(user_id, device_id, token, None, None)
            .await
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

    pub async fn device_exists(&self, user_id: &UserId, device_id: &DeviceId) -> bool {
        self.db
            .userdeviceid_metadata
            .contains(&(user_id, device_id))
            .await
    }

    /// Whether the device was signed in through the OIDC server.
    pub async fn is_oidc_device(&self, user_id: &UserId, device_id: &DeviceId) -> bool {
        self.db
            .oidcdevice_userdeviceid
            .contains(&(user_id, device_id))
            .await
    }

    /// The identity provider that authenticated this device, if it was one.
    /// A native (local account) OIDC device records none.
    pub async fn get_oidc_device_idp(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Option<String> {
        self.db
            .oidcdevice_userdeviceid
            .qry(&(user_id, device_id))
            .await
            .ok()
            .and_then(|idp| serde_json::from_slice::<String>(&idp).ok())
            .filter(|idp| !idp.is_empty())
    }

    /// Mark the device as signed in through the OIDC server, recording the
    /// identity provider that authenticated it (empty for a local account).
    pub fn mark_oidc_device(&self, user_id: &UserId, device_id: &DeviceId, idp_id: &str) {
        self.db
            .oidcdevice_userdeviceid
            .put((user_id, device_id), Json(idp_id))
            .ok();
    }
}
