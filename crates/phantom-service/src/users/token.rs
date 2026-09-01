use super::*;

impl Service {
    /// Creates an OpenID token, which can be used to prove that a user has
    /// access to an account (primarily for integrations)
    pub fn create_openid_token(&self, user_id: &UserId, token: &str) -> Result<u64> {
        use std::num::Saturating as Sat;

        let expires_in = self.services.server.config.openid_token_ttl;
        let expires_at = Sat(time::now_millis()) + Sat(expires_in) * Sat(1000);

        let mut value = expires_at.0.to_be_bytes().to_vec();
        value.extend_from_slice(user_id.as_bytes());

        self.db
            .openidtoken_expiresatuserid
            .insert(token.as_bytes(), value.as_slice())
            .ok();

        Ok(expires_in)
    }

    /// Find out which user an OpenID access token belongs to.
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

        OwnedUserId::try_from(user_string)
            .map_err(|e| err!(Database("User ID in openid_userid is invalid. {e}")))
    }

    /// Creates a short-lived login token, which can be used to log in using the
    /// `m.login.token` mechanism.
    pub fn create_login_token(&self, user_id: &UserId, token: &str) -> u64 {
        use std::num::Saturating as Sat;

        let expires_in = self.services.server.config.login_token_ttl;
        let expires_at = Sat(time::now_millis()) + Sat(expires_in);

        let value = (expires_at.0, user_id);
        self.db
            .logintoken_expiresatuserid
            .raw_put(token, value)
            .ok();

        expires_in
    }

    /// Find out which user a login token belongs to.
    /// Removes the token to prevent double-use attacks.
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
