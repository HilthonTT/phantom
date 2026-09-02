use super::*;

impl Service {
    pub async fn add_one_time_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        one_time_key_key: &KeyId<OneTimeKeyAlgorithm, OneTimeKeyName>,
        one_time_key_value: &Raw<OneTimeKey>,
    ) -> Result {
        let key = (user_id, device_id);
        if self.db.userdeviceid_metadata.qry(&key).await.is_err() {
            return Err!(Database(error!(
                ?user_id,
                ?device_id,
                message = "User does not exist or device has no metadata."
            )));
        }

        let mut key = user_id.as_bytes().to_vec();
        key.push(0xFF);
        key.extend_from_slice(device_id.as_bytes());
        key.push(0xFF);
        key.extend_from_slice(
            serde_json::to_string(one_time_key_key)
                .expect("DeviceKeyId::to_string always works")
                .as_bytes(),
        );

        self.db
            .onetimekeyid_onetimekeys
            .raw_put(key, Json(one_time_key_value))
            .ok();

        let count = self.services.server_state.next_count().unwrap();
        self.db
            .userid_lastonetimekeyupdate
            .raw_put(user_id, count)
            .ok();

        Ok(())
    }

    pub async fn last_one_time_keys_update(&self, user_id: &UserId) -> u64 {
        self.db
            .userid_lastonetimekeyupdate
            .get(user_id)
            .await
            .deserialized()
            .unwrap_or(0)
    }

    pub async fn take_one_time_key(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        key_algorithm: &OneTimeKeyAlgorithm,
    ) -> Result<(
        OwnedKeyId<OneTimeKeyAlgorithm, OneTimeKeyName>,
        Raw<OneTimeKey>,
    )> {
        let count = self.services.server_state.next_count()?.to_be_bytes();
        self.db
            .userid_lastonetimekeyupdate
            .insert(user_id, count)
            .ok();

        let mut prefix = user_id.as_bytes().to_vec();
        prefix.push(0xFF);
        prefix.extend_from_slice(device_id.as_bytes());
        prefix.push(0xFF);
        prefix.push(b'"');
        prefix.extend_from_slice(key_algorithm.as_ref().as_bytes());
        prefix.push(b':');

        let one_time_key = self
            .db
            .onetimekeyid_onetimekeys
            .raw_stream_prefix(&prefix)
            .ignore_err()
            .map(|(key, val)| {
                self.db.onetimekeyid_onetimekeys.remove(key).ok();

                let key = key
                    .rsplit(|&b| b == 0xFF)
                    .next()
                    .ok_or_else(|| err!(Database("OneTimeKeyId in db is invalid.")))
                    .unwrap();

                let key = serde_json::from_slice(key)
                    .map_err(|e| err!(Database("OneTimeKeyId in db is invalid. {e}")))
                    .unwrap();

                let val = serde_json::from_slice(val)
                    .map_err(|e| err!(Database("OneTimeKeys in db are invalid. {e}")))
                    .unwrap();

                (key, val)
            })
            .next()
            .await;

        one_time_key.ok_or_else(|| err!(Request(NotFound("No one-time-key found"))))
    }

    pub async fn count_one_time_keys(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> BTreeMap<OneTimeKeyAlgorithm, UInt> {
        type KeyVal<'a> = ((Ignore, Ignore, &'a Unquoted), Ignore);

        let mut algorithm_counts = BTreeMap::<OneTimeKeyAlgorithm, _>::new();
        // With `Interfix` the prefix ends at the device id's separator;
        // without it device "A" also counted the keys of device "AB".
        let query = (user_id, device_id, Interfix);
        self.db
            .onetimekeyid_onetimekeys
            .stream_prefix(&query)
            .ignore_err()
            .ready_for_each(|((Ignore, Ignore, device_key_id), Ignore): KeyVal<'_>| {
                let one_time_key_id: &OneTimeKeyId = device_key_id
                    .as_str()
                    .try_into()
                    .expect("Invalid DeviceKeyID in database");

                let count: &mut UInt = algorithm_counts
                    .entry(one_time_key_id.algorithm())
                    .or_default();

                *count = count.saturating_add(1_u32.into());
            })
            .await;

        algorithm_counts
    }

    pub async fn add_device_keys(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        device_keys: &Raw<DeviceKeys>,
    ) {
        let key = (user_id, device_id);

        self.db.keyid_key.put(key, Json(device_keys)).ok();
        self.mark_device_key_update(user_id).await;
    }

    pub async fn add_cross_signing_keys(
        &self,
        user_id: &UserId,
        master_key: &Option<Raw<CrossSigningKey>>,
        self_signing_key: &Option<Raw<CrossSigningKey>>,
        user_signing_key: &Option<Raw<CrossSigningKey>>,
        notify: bool,
    ) -> Result<()> {
        let keys = [
            (master_key, &self.db.userid_masterkeyid, "Master"),
            (
                self_signing_key,
                &self.db.userid_selfsigningkeyid,
                "Self signing",
            ),
            (
                user_signing_key,
                &self.db.userid_usersigningkeyid,
                "User signing",
            ),
        ];

        for (key, index, what) in keys {
            let Some(key) = key else {
                continue;
            };

            let (public_key, _) = parse_cross_signing_key(key, what)?;

            let keyid = serialize_to_vec((user_id, &public_key))
                .expect("failed to serialize cross-signing key id");

            self.db
                .keyid_key
                .insert(&keyid, key.json().get().as_bytes())
                .ok();

            index.insert(user_id.as_bytes(), &keyid).ok();
        }

        if notify {
            self.mark_device_key_update(user_id).await;
        }

        Ok(())
    }

    pub async fn sign_key(
        &self,
        target_id: &UserId,
        key_id: &str,
        signature: (String, String),
        sender_id: &UserId,
    ) -> Result {
        let key = (target_id, key_id);

        let mut cross_signing_key: serde_json::Value = self
            .db
            .keyid_key
            .qry(&key)
            .await
            .map_err(|_| err!(Request(InvalidParam("Tried to sign nonexistent key"))))?
            .deserialized()
            .map_err(|e| err!(Database(debug_warn!("key in keyid_key is invalid: {e:?}"))))?;

        let signatures = cross_signing_key
            .get_mut("signatures")
            .ok_or_else(|| {
                err!(Database(debug_warn!(
                    "key in keyid_key has no signatures field"
                )))
            })?
            .as_object_mut()
            .ok_or_else(|| {
                err!(Database(debug_warn!(
                    "key in keyid_key has invalid signatures field."
                )))
            })?
            .entry(sender_id.to_string())
            .or_insert_with(|| serde_json::Map::new().into());

        signatures
            .as_object_mut()
            .ok_or_else(|| {
                err!(Database(debug_warn!(
                    "signatures in keyid_key for a user is invalid."
                )))
            })?
            .insert(signature.0, signature.1.into());

        let key = (target_id, key_id);
        self.db.keyid_key.put(key, Json(cross_signing_key)).ok();

        self.mark_device_key_update(target_id).await;

        Ok(())
    }

    #[inline]
    pub fn keys_changed<'a>(
        &'a self,
        user_id: &'a UserId,
        from: u64,
        to: Option<u64>,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        self.keys_changed_user_or_room(user_id.as_str(), from, to)
            .map(|(user_id, ..)| user_id)
    }

    #[inline]
    pub fn room_keys_changed<'a>(
        &'a self,
        room_id: &'a RoomId,
        from: u64,
        to: Option<u64>,
    ) -> impl Stream<Item = (&'a UserId, u64)> + Send + 'a {
        self.keys_changed_user_or_room(room_id.as_str(), from, to)
    }

    fn keys_changed_user_or_room<'a>(
        &'a self,
        user_or_room_id: &'a str,
        from: u64,
        to: Option<u64>,
    ) -> impl Stream<Item = (&'a UserId, u64)> + Send + 'a {
        type KeyVal<'a> = ((&'a str, u64), &'a str);

        let to = to.unwrap_or(u64::MAX);
        let start = (user_or_room_id, from.saturating_add(1));
        self.db
            .keychangeid_userid
            .stream_from(&start)
            .ignore_err()
            .ready_take_while(move |((prefix, count), _): &KeyVal<'_>| {
                *prefix == user_or_room_id && *count <= to
            })
            .map(|((_, count), user_id): KeyVal<'_>| {
                (
                    <&UserId>::try_from(user_id).expect("valid user id in db"),
                    count,
                )
            })
    }

    pub async fn mark_device_key_update(&self, user_id: &UserId) {
        let count = self.services.server_state.next_count().unwrap();

        self.services
            .state_cache
            .rooms_joined(user_id)
            .filter(|room_id| self.services.state_accessor.is_encrypted_room(room_id))
            .ready_for_each(|room_id| {
                let key = (room_id, count);
                self.db.keychangeid_userid.put_raw(key, user_id).ok();
            })
            .await;

        let key = (user_id, count);
        self.db.keychangeid_userid.put_raw(key, user_id).ok();
    }

    pub async fn get_device_keys<'a>(
        &'a self,
        user_id: &'a UserId,
        device_id: &DeviceId,
    ) -> Result<Raw<DeviceKeys>> {
        let key_id = (user_id, device_id);
        self.db.keyid_key.qry(&key_id).await.deserialized()
    }

    pub async fn get_key<F>(
        &self,
        key_id: &[u8],
        sender_user: Option<&UserId>,
        user_id: &UserId,
        allowed_signatures: &F,
    ) -> Result<Raw<CrossSigningKey>>
    where
        F: Fn(&UserId) -> bool + Send + Sync,
    {
        let key: serde_json::Value = self.db.keyid_key.get(key_id).await.deserialized()?;

        let cleaned = clean_signatures(key, sender_user, user_id, allowed_signatures)?;
        let raw_value = serde_json::value::to_raw_value(&cleaned)?;
        Ok(Raw::from_json(raw_value))
    }

    pub async fn get_master_key<F>(
        &self,
        sender_user: Option<&UserId>,
        user_id: &UserId,
        allowed_signatures: &F,
    ) -> Result<Raw<CrossSigningKey>>
    where
        F: Fn(&UserId) -> bool + Send + Sync,
    {
        let key_id = self.db.userid_masterkeyid.get(user_id).await?;

        self.get_key(&key_id, sender_user, user_id, allowed_signatures)
            .await
    }

    pub async fn get_self_signing_key<F>(
        &self,
        sender_user: Option<&UserId>,
        user_id: &UserId,
        allowed_signatures: &F,
    ) -> Result<Raw<CrossSigningKey>>
    where
        F: Fn(&UserId) -> bool + Send + Sync,
    {
        let key_id = self.db.userid_selfsigningkeyid.get(user_id).await?;

        self.get_key(&key_id, sender_user, user_id, allowed_signatures)
            .await
    }

    pub async fn get_user_signing_key(&self, user_id: &UserId) -> Result<Raw<CrossSigningKey>> {
        self.db
            .userid_usersigningkeyid
            .get(user_id)
            .and_then(|key_id| self.db.keyid_key.get(&*key_id))
            .await
            .deserialized()
    }
}

/// The one public key a cross-signing key carries, with the key itself.
///
/// The spec allows exactly one, and the column layout depends on it: the
/// public key is what the key is stored under, so a second one would be
/// silently dropped rather than stored beside the first.
///
/// `what` names the key in the errors — "Master", "Self signing", "User
/// signing" — since all three are parsed through here.
pub fn parse_cross_signing_key(
    key: &Raw<CrossSigningKey>,
    what: &str,
) -> Result<(String, CrossSigningKey)> {
    let key: CrossSigningKey = key
        .deserialize()
        .map_err(|e| err!(Request(InvalidParam("Invalid {what} key: {e}"))))?;

    let mut public_keys = key.keys.values();
    let public_key = public_keys
        .next()
        .ok_or_else(|| err!(Request(InvalidParam("{what} key contained no key."))))?
        .clone();

    if public_keys.next().is_some() {
        return Err!(Request(InvalidParam(
            "{what} key contained more than one key."
        )));
    }

    Ok((public_key, key))
}

/// The key `master_key` is stored under, with the key itself.
pub fn parse_master_key(
    user_id: &UserId,
    master_key: &Raw<CrossSigningKey>,
) -> Result<(Vec<u8>, CrossSigningKey)> {
    let (public_key, master_key) = parse_cross_signing_key(master_key, "Master")?;
    let keyid = serialize_to_vec((user_id, &public_key))?;

    Ok((keyid, master_key))
}

/// The public key of `user_signing_key`.
pub fn parse_user_signing_key(user_signing_key: &Raw<CrossSigningKey>) -> Result<String> {
    parse_cross_signing_key(user_signing_key, "User signing").map(at!(0))
}

/// Ensure that a user only sees signatures from themselves and the target user
fn clean_signatures<F>(
    mut cross_signing_key: serde_json::Value,
    sender_user: Option<&UserId>,
    user_id: &UserId,
    allowed_signatures: &F,
) -> Result<serde_json::Value>
where
    F: Fn(&UserId) -> bool + Send + Sync,
{
    if let Some(signatures) = cross_signing_key
        .get_mut("signatures")
        .and_then(|v| v.as_object_mut())
    {
        let new_capacity = signatures.len() / 2;
        for (user, signature) in
            mem::replace(signatures, serde_json::Map::with_capacity(new_capacity))
        {
            let sid = <&UserId>::try_from(user.as_str())
                .map_err(|_| Error::bad_database("Invalid user ID in database."))?;
            if sender_user == Some(user_id) || sid == user_id || allowed_signatures(sid) {
                signatures.insert(user, signature);
            }
        }
    }

    Ok(cross_signing_key)
}
