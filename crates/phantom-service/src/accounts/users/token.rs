use super::*;

impl Service {
    pub fn create_openid_token(&self, user_id: &UserId, token: &str) -> Result<u64> {
        use std::num::Saturating as Sat;

        let expires_in = self.services.server.config.auth.openid_token_ttl;
        let expires_at = Sat(time::now_millis()) + Sat(expires_in) * Sat(1000);

        let mut value = expires_at.0.to_be_bytes().to_vec();
        value.extend_from_slice(user_id.as_bytes());

        self.db
            .openidtoken_expiresatuserid
            .insert(token.as_bytes(), value.as_slice())
            .ok();

        Ok(expires_in)
    }

    pub async fn find_from_openid_token(&self, token: &str) -> Result<OwnedUserId> {
        let Ok(value) = self.db.openidtoken_expiresatuserid.get(token).await else {
            return Err!(Request(Unauthorized("OpenID token is unrecognised")));
        };

        let (expires_at_bytes, user_bytes) = value.split_at(0_u64.to_be_bytes().len());
        let expires_at = u64::from_be_bytes(
            expires_at_bytes
                .try_into()
                .map_err(|e| err!(Database("expires_at in openid_userid is invalid u64. {e}")))?,
        );

        if expires_at < time::now_millis() {
            debug_warn!("OpenID token is expired, removing");
            self.db
                .openidtoken_expiresatuserid
                .remove(token.as_bytes())
                .ok();

            return Err!(Request(Unauthorized("OpenID token is expired")));
        }

        let user_string = text::string_from_bytes(user_bytes)
            .map_err(|e| err!(Database("User ID in openid_userid is invalid unicode. {e}")))?;

        let user_id = OwnedUserId::try_from(user_string)
            .map_err(|e| err!(Database("User ID in openid_userid is invalid. {e}")))?;

        // Deactivation doesn't purge these, so don't vouch for the user past it.
        if !self.is_active(&user_id).await {
            return Err!(Request(Unauthorized("OpenID token is unrecognised")));
        }

        Ok(user_id)
    }

    pub fn create_login_token(&self, user_id: &UserId, token: &str) -> u64 {
        use std::num::Saturating as Sat;

        let expires_in = self.services.server.config.logging.login_token_ttl;
        let expires_at = Sat(time::now_millis()) + Sat(expires_in);

        let value = (expires_at.0, user_id);
        self.db
            .logintoken_expiresatuserid
            .raw_put(token, value)
            .ok();

        expires_in
    }

    /// Verify a login token is valid and return its owner without consuming it.
    /// Unlike `find_from_login_token`, the token remains in the database
    /// after this call and can still be consumed later.
    pub async fn peek_login_token(&self, token: &str) -> Result<OwnedUserId> {
        let Ok(value) = self.db.logintoken_expiresatuserid.get(token).await else {
            return Err!(Request(Forbidden("Login token is unrecognised")));
        };
        let (expires_at, user_id): (u64, OwnedUserId) = value.deserialized()?;

        if expires_at < time::now_millis() {
            trace!(?user_id, ?token, "Removing expired login token");

            self.db.logintoken_expiresatuserid.remove(token).ok();

            return Err!(Request(Forbidden("Login token is expired")));
        }

        Ok(user_id)
    }

    pub async fn find_from_login_token(&self, token: &str) -> Result<OwnedUserId> {
        let Ok(value) = self.db.logintoken_expiresatuserid.get(token).await else {
            return Err!(Request(Forbidden("Login token is unrecognised")));
        };
        let (expires_at, user_id): (u64, OwnedUserId) = value.deserialized()?;

        if expires_at < time::now_millis() {
            trace!(?user_id, ?token, "Removing expired login token");

            self.db.logintoken_expiresatuserid.remove(token).ok();

            return Err!(Request(Forbidden("Login token is expired")));
        }

        self.db.logintoken_expiresatuserid.remove(token).ok();

        Ok(user_id)
    }
}

/// Length of a generated access token.
pub const TOKEN_LENGTH: usize = 32;

/// Prefix marking a refresh token, so one is never mistaken for an access
/// token although both resolve through `token_userdeviceid`.
const REFRESH_TOKEN_PREFIX: &str = "refresh_";

/// Classification of a refresh token presented for rotation at a token
/// endpoint.
pub enum RefreshToken {
    /// The device's current refresh token; rotate it.
    Current {
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        expires_at: Option<SystemTime>,
    },

    /// A spent (already-rotated) token retained for one generation. `grace` is
    /// set when its successor is still current and it was spent within the
    /// configured window, marking a benign double-submit rather than a replay;
    /// `current` is the successor for which to re-issue an access token.
    Replayed {
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        current: String,
        grace: bool,
    },

    /// Not a recognised refresh token.
    Unknown,
}

#[must_use]
pub fn generate_refresh_token() -> String {
    format!("{REFRESH_TOKEN_PREFIX}{}", rand::string(TOKEN_LENGTH))
}

#[must_use]
pub fn is_refresh_token(token: &str) -> bool {
    token.starts_with(REFRESH_TOKEN_PREFIX)
}

impl Service {
    /// Find out which user an access or refresh token belongs to, and when it
    /// expires.
    pub async fn find_from_token(
        &self,
        token: &str,
    ) -> Result<(OwnedUserId, OwnedDeviceId, Option<SystemTime>)> {
        let value = self.db.token_userdeviceid.get(token).await?;

        let (user_id, device_id, expires_at) =
            deserialize::<(OwnedUserId, OwnedDeviceId, Option<u64>)>(&value)
                // Tokens written before expiry was recorded carry no third field.
                .or_else(|_| {
                    deserialize::<(OwnedUserId, OwnedDeviceId)>(&value)
                        .map(|(user_id, device_id)| (user_id, device_id, None))
                })?;

        let expires_at = expires_at.map(|secs| UNIX_EPOCH + Duration::from_secs(secs));

        Ok((user_id, device_id, expires_at))
    }

    /// Mint an access token, expiring after `access_token_ttl` when `expires`.
    #[must_use]
    pub fn generate_access_token(&self, expires: bool) -> (String, Option<Duration>) {
        let access_token = rand::string(TOKEN_LENGTH);
        let expires_in = expires
            .then_some(self.services.server.config.auth.access_token_ttl)
            .map(Duration::from_secs);

        (access_token, expires_in)
    }

    /// Replace the access token of one device, and its refresh token when one
    /// is given.
    pub async fn set_access_token(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        access_token: &str,
        expires_in: Option<Duration>,
        refresh_token: Option<&str>,
    ) -> Result {
        let userdeviceid = (user_id, device_id);
        if self
            .db
            .userdeviceid_metadata
            .qry(&userdeviceid)
            .await
            .is_err()
        {
            return Err!(Database(error!(
                ?user_id,
                ?device_id,
                message = "User does not exist or device has no metadata."
            )));
        }

        if let Some(refresh_token) = refresh_token {
            self.set_refresh_token(user_id, device_id, refresh_token)
                .await?;
        }

        let expires_at = expires_in
            .map(time::timepoint_from_now)
            .transpose()?
            .map(time::duration_since_epoch)
            .as_ref()
            .map(Duration::as_secs);

        let previous = self.db.userdeviceid_token.qry(&userdeviceid).await;

        let mut txn = self.db.txn();

        if let Ok(previous) = previous {
            txn.remove(&self.db.token_userdeviceid, &previous);
        }

        let value = serialize_val((user_id, device_id, expires_at))?;

        txn.insert(&self.db.token_userdeviceid, access_token, value);
        txn.put_raw(&self.db.userdeviceid_token, userdeviceid, access_token)?;

        txn.execute()
    }

    /// Revoke both the access and the refresh token of one device, without
    /// deleting the device.
    pub async fn remove_tokens(&self, user_id: &UserId, device_id: &DeviceId) {
        self.remove_access_token(user_id, device_id).await.ok();
        self.remove_refresh_token(user_id, device_id).await.ok();
    }

    /// Revoke the access token of one device, without deleting the device.
    pub async fn remove_access_token(&self, user_id: &UserId, device_id: &DeviceId) -> Result {
        let userdeviceid = (user_id, device_id);
        let token = self.db.userdeviceid_token.qry(&userdeviceid).await;

        let mut txn = self.db.txn();

        if let Ok(token) = token {
            txn.remove(&self.db.token_userdeviceid, &token);
        }

        txn.del(&self.db.userdeviceid_token, userdeviceid)?;

        txn.execute()
    }

    /// Replace the refresh token of one device, retaining the outgoing one for
    /// a generation so a later replay is detectable.
    pub async fn set_refresh_token(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        refresh_token: &str,
    ) -> Result {
        debug_assert!(
            is_refresh_token(refresh_token),
            "refresh_token missing prefix"
        );

        let config = &self.services.server.config.auth;
        let ttl = config.refresh_token_ttl;
        let idle_only = config.refresh_token_idle_only;

        // Absolute mode carries the prior deadline forward instead of sliding it.
        let prior_expires_at = if ttl != 0 && !idle_only {
            self.find_refresh_token_expires_at(user_id, device_id).await
        } else {
            None
        };

        let userdeviceid = (user_id, device_id);
        let spent: Option<String> = self
            .db
            .userdeviceid_refresh
            .qry(&userdeviceid)
            .await
            .deserialized()
            .ok();

        // Also drops the prior spent entry.
        self.remove_refresh_token(user_id, device_id).await?;

        let expires_at = match (ttl, prior_expires_at) {
            (0, _) => None,
            (_, Some(prior)) => Some(prior),
            (ttl, None) => Some(time::timepoint_from_now(Duration::from_secs(ttl))?),
        };

        let expires_at = expires_at
            .map(time::duration_since_epoch)
            .as_ref()
            .map(Duration::as_secs);

        let mut txn = self.db.txn();

        let value = serialize_val((user_id, device_id, expires_at))?;
        txn.insert(&self.db.token_userdeviceid, refresh_token, value);
        txn.put_raw(&self.db.userdeviceid_refresh, userdeviceid, refresh_token)?;

        // Retain the outgoing token as the device's spent token, pointing at its
        // successor so a double-submit can be told apart from a replay.
        if let Some(spent) = spent {
            let spent_at = time::now_secs();
            let value = serialize_val((user_id, device_id, refresh_token, spent_at))?;

            txn.insert(&self.db.spentrefresh_userdeviceid, &spent, value);
            txn.put_raw(&self.db.userdeviceid_spentrefresh, userdeviceid, &spent)?;
        }

        txn.execute()
    }

    /// The expiry stored alongside the device's current refresh token, if any.
    async fn find_refresh_token_expires_at(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Option<SystemTime> {
        let token: String = self
            .db
            .userdeviceid_refresh
            .qry(&(user_id, device_id))
            .await
            .deserialized()
            .ok()?;

        self.find_from_token(&token).await.ok()?.2
    }

    /// Revoke the refresh token, and the spent one retained beside it, without
    /// deleting the device.
    pub async fn remove_refresh_token(&self, user_id: &UserId, device_id: &DeviceId) -> Result {
        let userdeviceid = (user_id, device_id);
        let refresh_token = self.db.userdeviceid_refresh.qry(&userdeviceid).await;
        let spent = self.db.userdeviceid_spentrefresh.qry(&userdeviceid).await;

        let mut txn = self.db.txn();

        if let Ok(refresh_token) = refresh_token {
            txn.remove(&self.db.token_userdeviceid, &refresh_token);
        }

        if let Ok(spent) = spent {
            txn.remove(&self.db.spentrefresh_userdeviceid, &spent);
        }

        txn.del(&self.db.userdeviceid_refresh, userdeviceid)?;
        txn.del(&self.db.userdeviceid_spentrefresh, userdeviceid)?;

        txn.execute()
    }

    /// Classify a presented refresh token for the token-endpoint rotation path.
    pub async fn classify_refresh_token(&self, presented: &str) -> RefreshToken {
        if !is_refresh_token(presented) {
            return RefreshToken::Unknown;
        }

        // The current refresh token resolves and matches the device's pointer.
        if let Ok((user_id, device_id, expires_at)) = self.find_from_token(presented).await {
            let current: Option<String> = self
                .db
                .userdeviceid_refresh
                .qry(&(&user_id, &device_id))
                .await
                .deserialized()
                .ok();

            if current.as_deref() == Some(presented) {
                return RefreshToken::Current {
                    user_id,
                    device_id,
                    expires_at,
                };
            }
        }

        // Otherwise it may be the one retained spent token: a benign
        // double-submit inside the grace window, or a replay to treat as a
        // compromise.
        let Ok((user_id, device_id, successor, spent_at)) = self
            .db
            .spentrefresh_userdeviceid
            .get(presented)
            .await
            .deserialized::<(OwnedUserId, OwnedDeviceId, String, u64)>()
        else {
            return RefreshToken::Unknown;
        };

        let current: Option<String> = self
            .db
            .userdeviceid_refresh
            .qry(&(&user_id, &device_id))
            .await
            .deserialized()
            .ok();

        let grace_window = self.services.server.config.auth.refresh_token_reuse_grace;
        let elapsed = time::now_secs().saturating_sub(spent_at);
        let grace = grace_window != 0
            && elapsed <= grace_window
            && current.as_deref() == Some(successor.as_str());

        RefreshToken::Replayed {
            user_id,
            device_id,
            current: successor,
            grace,
        }
    }
}

#[cfg(test)]
mod tests {
    use phantom_database::{deserialize, serialize_val};
    use ruma::{OwnedDeviceId, OwnedUserId, device_id, user_id};

    type Value = (OwnedUserId, OwnedDeviceId, Option<u64>);

    #[test]
    fn token_value_round_trips_with_and_without_expiry() {
        let (user_id, device_id) = (user_id!("@alice:example.com"), device_id!("DEVICE"));

        for expires_at in [Some(1_700_000_000_u64), None] {
            let value = serialize_val((user_id, device_id, expires_at)).unwrap();
            let decoded: Value = deserialize(&value).unwrap();

            assert_eq!(
                decoded,
                (user_id.to_owned(), device_id.to_owned(), expires_at)
            );
        }
    }

    #[test]
    fn legacy_token_value_still_decodes() {
        let (user_id, device_id) = (user_id!("@alice:example.com"), device_id!("DEVICE"));
        let legacy = serialize_val((user_id, device_id)).unwrap();

        let decoded = deserialize::<Value>(&legacy).or_else(|_| {
            deserialize::<(OwnedUserId, OwnedDeviceId)>(&legacy).map(|(u, d)| (u, d, None))
        });

        assert_eq!(
            decoded.unwrap(),
            (user_id.to_owned(), device_id.to_owned(), None)
        );
    }
}
