use super::*;

impl Service {
    pub async fn displayname(&self, user_id: &UserId) -> Result<String> {
        self.db.userid_displayname.get(user_id).await.deserialized()
    }

    pub fn set_displayname(&self, user_id: &UserId, displayname: Option<String>) {
        if let Some(displayname) = displayname {
            self.db.userid_displayname.insert(user_id, displayname).ok();
        } else {
            self.db.userid_displayname.remove(user_id).ok();
        }
    }

    pub async fn avatar_url(&self, user_id: &UserId) -> Result<OwnedMxcUri> {
        self.db.userid_avatarurl.get(user_id).await.deserialized()
    }

    pub fn set_avatar_url(&self, user_id: &UserId, avatar_url: Option<OwnedMxcUri>) {
        match avatar_url {
            Some(avatar_url) => {
                self.db.userid_avatarurl.insert(user_id, &avatar_url).ok();
            }
            _ => {
                self.db.userid_avatarurl.remove(user_id).ok();
            }
        }
    }

    pub async fn blurhash(&self, user_id: &UserId) -> Result<String> {
        self.db.userid_blurhash.get(user_id).await.deserialized()
    }

    pub fn set_blurhash(&self, user_id: &UserId, blurhash: Option<String>) {
        if let Some(blurhash) = blurhash {
            self.db.userid_blurhash.insert(user_id, blurhash).ok();
        } else {
            self.db.userid_blurhash.remove(user_id).ok();
        }
    }

    pub async fn profile_key(
        &self,
        user_id: &UserId,
        profile_key: &str,
    ) -> Result<serde_json::Value> {
        let key = (user_id, profile_key);
        self.db
            .useridprofilekey_value
            .qry(&key)
            .await
            .deserialized()
    }

    pub fn all_profile_keys<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = (String, serde_json::Value)> + 'a + Send {
        type KeyVal = ((Ignore, String), serde_json::Value);

        let prefix = (user_id, Interfix);
        self.db
            .useridprofilekey_value
            .stream_prefix(&prefix)
            .ignore_err()
            .map(|((_, key), val): KeyVal| (key, val))
    }

    pub fn set_profile_key(
        &self,
        user_id: &UserId,
        profile_key: &str,
        profile_key_value: Option<serde_json::Value>,
    ) {
        let key = (user_id, profile_key);

        if let Some(value) = profile_key_value {
            self.db.useridprofilekey_value.put(key, Json(value)).ok();
        } else {
            self.db.useridprofilekey_value.del(key).ok();
        }
    }

    pub async fn clear_profile(&self, user_id: &UserId) {
        self.set_displayname(user_id, None);
        self.set_avatar_url(user_id, None);
        self.set_blurhash(user_id, None);

        self.db
            .useridprofilekey_value
            .del_prefix(&(user_id, Interfix))
            .await;
    }

    pub async fn timezone(&self, user_id: &UserId) -> Result<String> {
        let unstable_key = (user_id, "us.cloke.msc4175.tz");
        let stable_key = (user_id, "m.tz");
        self.db
            .useridprofilekey_value
            .qry(&unstable_key)
            .or_else(|_| self.db.useridprofilekey_value.qry(&stable_key))
            .await
            .deserialized()
    }

    pub fn set_timezone(&self, user_id: &UserId, timezone: Option<String>) {
        let key = (user_id, "us.cloke.msc4175.tz");

        if let Some(timezone) = timezone {
            self.db.useridprofilekey_value.put_raw(key, &timezone).ok();
        } else {
            self.db.useridprofilekey_value.del(key).ok();
        }
    }
}
