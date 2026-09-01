mod device;
mod keys;
mod profile;
mod to_device;
mod token;

use std::{collections::BTreeMap, mem, sync::Arc};

use futures::{Stream, StreamExt, TryFutureExt};
use phantom_core::{
    Err, Error, Result, at, bytes, debug_warn, err, hash, rand,
    server::Server,
    stream::{ReadyExt, TryIgnore},
    text::{self, Unquoted},
    time, trace,
};
use phantom_database::{Deserialized, Ignore, Interfix, Json, Map, serialize_to_vec};
use ruma::{
    DeviceId, KeyId, MilliSecondsSinceUnixEpoch, OneTimeKeyAlgorithm, OneTimeKeyId, OneTimeKeyName,
    OwnedDeviceId, OwnedKeyId, OwnedMxcUri, OwnedUserId, RoomId, UInt, UserId,
    api::client::{device::Device, filter::FilterDefinition},
    encryption::{CrossSigningKey, DeviceKeys, OneTimeKey},
    events::{
        AnyToDeviceEvent, GlobalAccountDataEventType, ignored_user_list::IgnoredUserListEvent,
    },
    serde::Raw,
};
use serde_json::json;

pub use self::keys::{parse_cross_signing_key, parse_master_key, parse_user_signing_key};
use crate::{Dep, account_data, rooms, server_state};

pub struct Service {
    services: Services,
    db: Data,
}

struct Services {
    server: Arc<Server>,
    account_data: Dep<account_data::Service>,
    server_state: Dep<server_state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
}

struct Data {
    keychangeid_userid: Arc<Map>,
    keyid_key: Arc<Map>,
    onetimekeyid_onetimekeys: Arc<Map>,
    openidtoken_expiresatuserid: Arc<Map>,
    logintoken_expiresatuserid: Arc<Map>,
    todeviceid_events: Arc<Map>,
    token_userdeviceid: Arc<Map>,
    userdeviceid_metadata: Arc<Map>,
    userdeviceid_token: Arc<Map>,
    userfilterid_filter: Arc<Map>,
    userid_avatarurl: Arc<Map>,
    userid_blurhash: Arc<Map>,
    userid_devicelistversion: Arc<Map>,
    userid_displayname: Arc<Map>,
    userid_lastonetimekeyupdate: Arc<Map>,
    userid_masterkeyid: Arc<Map>,
    userid_password: Arc<Map>,
    userid_selfsigningkeyid: Arc<Map>,
    userid_usersigningkeyid: Arc<Map>,
    useridprofilekey_value: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                account_data: args.depend::<account_data::Service>("account_data"),
                server_state: args.depend::<server_state::Service>("server_state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
            },
            db: Data {
                keychangeid_userid: args.db["keychangeid_userid"].clone(),
                keyid_key: args.db["keyid_key"].clone(),
                onetimekeyid_onetimekeys: args.db["onetimekeyid_onetimekeys"].clone(),
                openidtoken_expiresatuserid: args.db["openidtoken_expiresatuserid"].clone(),
                logintoken_expiresatuserid: args.db["logintoken_expiresatuserid"].clone(),
                todeviceid_events: args.db["todeviceid_events"].clone(),
                token_userdeviceid: args.db["token_userdeviceid"].clone(),
                userdeviceid_metadata: args.db["userdeviceid_metadata"].clone(),
                userdeviceid_token: args.db["userdeviceid_token"].clone(),
                userfilterid_filter: args.db["userfilterid_filter"].clone(),
                userid_avatarurl: args.db["userid_avatarurl"].clone(),
                userid_blurhash: args.db["userid_blurhash"].clone(),
                userid_devicelistversion: args.db["userid_devicelistversion"].clone(),
                userid_displayname: args.db["userid_displayname"].clone(),
                userid_lastonetimekeyupdate: args.db["userid_lastonetimekeyupdate"].clone(),
                userid_masterkeyid: args.db["userid_masterkeyid"].clone(),
                userid_password: args.db["userid_password"].clone(),
                userid_selfsigningkeyid: args.db["userid_selfsigningkeyid"].clone(),
                userid_usersigningkeyid: args.db["userid_usersigningkeyid"].clone(),
                useridprofilekey_value: args.db["useridprofilekey_value"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Returns true/false based on whether the recipient/receiving user has
    /// blocked the sender
    pub async fn user_is_ignored(&self, sender_user: &UserId, recipient_user: &UserId) -> bool {
        self.services
            .account_data
            .get_global(recipient_user, GlobalAccountDataEventType::IgnoredUserList)
            .await
            .is_ok_and(|ignored: IgnoredUserListEvent| {
                ignored
                    .content
                    .ignored_users
                    .keys()
                    .any(|blocked_user| blocked_user == sender_user)
            })
    }

    /// Whether a user is an admin, which is to say joined to the admin room.
    ///
    /// The reference asks its admin service, but that service answers the
    /// question the same way — membership of the admin room is what being an
    /// admin *is*, not a flag kept beside it.
    pub async fn is_admin(&self, user_id: &UserId) -> bool {
        let Ok(admin_room) = self.services.server_state.admin_room_id().await else {
            return false;
        };

        self.services
            .state_cache
            .is_joined(user_id, &admin_room)
            .await
    }

    /// Create a new user account on this homeserver.
    #[inline]
    pub fn create(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
        self.set_password(user_id, password)
    }

    /// Deactivate account
    pub async fn deactivate_account(&self, user_id: &UserId) -> Result<()> {
        self.all_device_ids(user_id)
            .for_each(|device_id| self.remove_device(user_id, device_id))
            .await;

        self.set_password(user_id, None)?;

        Ok(())
    }

    /// Check if a user has an account on this homeserver.
    #[inline]
    pub async fn exists(&self, user_id: &UserId) -> bool {
        self.db.userid_password.get(user_id).await.is_ok()
    }

    /// Check if account is deactivated
    pub async fn is_deactivated(&self, user_id: &UserId) -> Result<bool> {
        self.db
            .userid_password
            .get(user_id)
            .map_ok(|val| val.is_empty())
            .map_err(|_| err!(Request(NotFound("User does not exist."))))
            .await
    }

    /// Check if account is active, infallible
    pub async fn is_active(&self, user_id: &UserId) -> bool {
        !self.is_deactivated(user_id).await.unwrap_or(true)
    }

    /// Check if account is active, infallible
    pub async fn is_active_local(&self, user_id: &UserId) -> bool {
        self.services.server_state.user_is_local(user_id) && self.is_active(user_id).await
    }

    /// Returns the number of users registered on this server.
    #[inline]
    pub async fn count(&self) -> usize {
        self.db.userid_password.count().await
    }

    /// Find out which user an access token belongs to.
    pub async fn find_from_token(&self, token: &str) -> Result<(OwnedUserId, OwnedDeviceId)> {
        self.db.token_userdeviceid.get(token).await.deserialized()
    }

    /// Returns an iterator over all users on this homeserver (offered for
    /// compatibility)
    #[allow(clippy::iter_without_into_iter, clippy::iter_not_returning_iterator)]
    pub fn iter(&self) -> impl Stream<Item = OwnedUserId> + Send + '_ {
        self.stream().map(ToOwned::to_owned)
    }

    /// Returns an iterator over all users on this homeserver.
    pub fn stream(&self) -> impl Stream<Item = &UserId> + Send {
        self.db
            .userid_password
            .keys::<&str>()
            .ignore_err()
            .map(|user_id| <&UserId>::try_from(user_id).expect("valid user id in db"))
    }

    /// Returns a list of local users as list of usernames.
    ///
    /// A user account is considered `local` if the length of it's password is
    /// greater then zero.
    pub fn list_local_users(&self) -> impl Stream<Item = &UserId> + Send + '_ {
        self.db
            .userid_password
            .stream()
            .ignore_err()
            .ready_filter_map(|(u, p): (&str, &[u8])| {
                (!p.is_empty()).then(|| <&UserId>::try_from(u).expect("valid user id in db"))
            })
    }

    /// Returns the password hash for the given user.
    pub async fn password_hash(&self, user_id: &UserId) -> Result<String> {
        self.db.userid_password.get(user_id).await.deserialized()
    }

    /// Hash and set the user's password to the Argon2 hash
    pub fn set_password(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
        password
            .map(hash::password)
            .transpose()
            .map_err(|e| {
                err!(Request(InvalidParam(
                    "Password does not meet the requirements: {e}"
                )))
            })?
            .map_or_else(
                || self.db.userid_password.insert(user_id, b""),
                |hash| self.db.userid_password.insert(user_id, hash),
            )
    }

    /// Creates a new sync filter. Returns the filter id.
    pub fn create_filter(&self, user_id: &UserId, filter: &FilterDefinition) -> String {
        let filter_id = rand::string(4);

        let key = (user_id, &filter_id);
        self.db.userfilterid_filter.put(key, Json(filter)).ok();

        filter_id
    }

    pub async fn get_filter(&self, user_id: &UserId, filter_id: &str) -> Result<FilterDefinition> {
        let key = (user_id, filter_id);
        self.db.userfilterid_filter.qry(&key).await.deserialized()
    }
}

fn increment(db: &Arc<Map>, key: &[u8]) {
    let old = db.get_blocking(key);
    let new = bytes::increment(old.ok().as_deref());
    db.insert(key, new).ok();
}
