mod device;
mod keys;
#[cfg(test)]
mod tests;
mod to_device;
mod token;

use std::{
    collections::{BTreeMap, HashMap},
    mem,
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::{Stream, StreamExt, TryFutureExt};
use phantom_core::{
    Err, Error, Result, at, bytes, debug_warn, err, hash, rand,
    runtime::server::Server,
    stream::{ReadyExt, TryIgnore},
    sync::MutexMap,
    text::{self, Unquoted},
    time, trace,
};
use phantom_database::{
    Database, Deserialized, Ignore, Interfix, Json, Map, Txn, deserialize, serialize_to_vec,
    serialize_val, table,
};
use ruma::{
    DeviceId, KeyId, MilliSecondsSinceUnixEpoch, OneTimeKeyAlgorithm, OneTimeKeyId, OneTimeKeyName,
    OwnedDeviceId, OwnedKeyId, OwnedUserId, RoomId, UInt, UserId,
    api::client::{device::Device, filter::FilterDefinition},
    encryption::{CrossSigningKey, DeviceKeys, OneTimeKey},
    events::{
        AnyToDeviceEvent, GlobalAccountDataEventType, ignored_user_list::IgnoredUserListEvent,
    },
    serde::Raw,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use self::{
    keys::{parse_cross_signing_key, parse_master_key, parse_user_signing_key},
    token::{RefreshToken, TOKEN_LENGTH, generate_refresh_token, is_refresh_token},
};
use crate::{Dep, accounts, accounts::account_data, ops::server_state, rooms};

pub struct Service {
    services: Services,
    db: Data,
    /// Users who approved a cross-signing reset, and when that approval
    /// lapses. Kept in memory only: a restart simply revokes the approval.
    cross_signing_replacement_allowed: RwLock<HashMap<OwnedUserId, Instant>>,
    /// Serializes one-time-key claims so a key is handed out at most once.
    one_time_key_locks: MutexMap<OwnedUserId, ()>,
}

/// How long an approved cross-signing reset stays open.
pub const CROSS_SIGNING_REPLACEMENT_TTL: Duration = Duration::from_secs(10 * 60);

/// The deactivation marker's value: when an account was deactivated, and who
/// asked for it when that was not the server itself.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Deactivation {
    pub deactivated_at: MilliSecondsSinceUnixEpoch,
    pub by: Option<OwnedUserId>,
}

struct Services {
    server: Arc<Server>,
    account_data: Dep<account_data::Service>,
    pusher: Dep<accounts::pusher::Service>,
    alias: Dep<rooms::alias::Service>,
    server_state: Dep<server_state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
}

struct Data {
    database: Arc<Database>,
    keychangeid_userid: Arc<Map>,
    keyid_key: Arc<Map>,
    oidcdevice_userdeviceid: Arc<Map>,
    onetimekeyid_onetimekeys: Arc<Map>,
    openidtoken_expiresatuserid: Arc<Map>,
    logintoken_expiresatuserid: Arc<Map>,
    spentrefresh_userdeviceid: Arc<Map>,
    todeviceid_events: Arc<Map>,
    token_userdeviceid: Arc<Map>,
    userdeviceid_metadata: Arc<Map>,
    userdeviceid_refresh: Arc<Map>,
    userdeviceid_spentrefresh: Arc<Map>,
    userdeviceid_token: Arc<Map>,
    userfilterid_filter: Arc<Map>,
    userid_deactivated: Arc<Map>,
    userid_devicelistversion: Arc<Map>,
    userid_lastonetimekeyupdate: Arc<Map>,
    userid_masterkeyid: Arc<Map>,
    userid_password: Arc<Map>,
    userid_selfsigningkeyid: Arc<Map>,
    userid_usersigningkeyid: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                account_data: args.depend::<account_data::Service>(),
                pusher: args.depend::<accounts::pusher::Service>(),
                server_state: args.depend::<server_state::Service>(),
                state_accessor: args.depend::<rooms::state_accessor::Service>(),
                alias: args.depend::<rooms::alias::Service>(),
                state_cache: args.depend::<rooms::state_cache::Service>(),
            },
            db: Data {
                database: args.db.clone(),
                keychangeid_userid: args.db[table::KEYCHANGEID_USERID].clone(),
                keyid_key: args.db[table::KEYID_KEY].clone(),
                oidcdevice_userdeviceid: args.db[table::OIDCDEVICE_USERDEVICEID].clone(),
                onetimekeyid_onetimekeys: args.db[table::ONETIMEKEYID_ONETIMEKEYS].clone(),
                openidtoken_expiresatuserid: args.db[table::OPENIDTOKEN_EXPIRESATUSERID].clone(),
                logintoken_expiresatuserid: args.db[table::LOGINTOKEN_EXPIRESATUSERID].clone(),
                spentrefresh_userdeviceid: args.db[table::SPENTREFRESH_USERDEVICEID].clone(),
                todeviceid_events: args.db[table::TODEVICEID_EVENTS].clone(),
                token_userdeviceid: args.db[table::TOKEN_USERDEVICEID].clone(),
                userdeviceid_metadata: args.db[table::USERDEVICEID_METADATA].clone(),
                userdeviceid_refresh: args.db[table::USERDEVICEID_REFRESH].clone(),
                userdeviceid_spentrefresh: args.db[table::USERDEVICEID_SPENTREFRESH].clone(),
                userdeviceid_token: args.db[table::USERDEVICEID_TOKEN].clone(),
                userfilterid_filter: args.db[table::USERFILTERID_FILTER].clone(),
                userid_deactivated: args.db[table::USERID_DEACTIVATED].clone(),
                userid_devicelistversion: args.db[table::USERID_DEVICELISTVERSION].clone(),
                userid_lastonetimekeyupdate: args.db[table::USERID_LASTONETIMEKEYUPDATE].clone(),
                userid_masterkeyid: args.db[table::USERID_MASTERKEYID].clone(),
                userid_password: args.db[table::USERID_PASSWORD].clone(),
                userid_selfsigningkeyid: args.db[table::USERID_SELFSIGNINGKEYID].clone(),
                userid_usersigningkeyid: args.db[table::USERID_USERSIGNINGKEYID].clone(),
            },
            cross_signing_replacement_allowed: RwLock::new(HashMap::new()),
            one_time_key_locks: MutexMap::new(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
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

    pub async fn is_admin(&self, user_id: &UserId) -> bool {
        let admin_alias = &self.services.server_state.admin_alias;
        let Ok(admin_room) = self.services.alias.resolve_local_alias(admin_alias).await else {
            return false;
        };

        self.services
            .state_cache
            .is_joined(user_id, &admin_room)
            .await
    }

    #[inline]
    pub fn create(&self, user_id: &UserId, password: Option<&str>) -> Result<()> {
        self.set_password(user_id, password)
    }

    /// Deactivates a local account: marks it deactivated, signs every device
    /// out and clears its password. `by` is who asked, kept with the marker
    /// for the operator; `None` is the server itself.
    ///
    /// The marker, not the empty password, is what makes the account
    /// deactivated. Guests and appservice users never had a password, and
    /// reading the empty password as deactivation is what made them look
    /// deactivated before the marker existed.
    pub async fn deactivate_account(&self, user_id: &UserId, by: Option<&UserId>) -> Result<()> {
        self.mark_deactivated(user_id, by)?;

        self.revoke_credentials(user_id).await
    }

    /// Sets the deactivation marker alone, for an account whose credentials
    /// are already gone.
    pub fn mark_deactivated(&self, user_id: &UserId, by: Option<&UserId>) -> Result<()> {
        let deactivation = Deactivation {
            deactivated_at: MilliSecondsSinceUnixEpoch::now(),
            by: by.map(ToOwned::to_owned),
        };

        self.db
            .userid_deactivated
            .raw_put(user_id, Json(&deactivation))
    }

    /// Signs every device out and clears the password, without deactivating.
    /// The server user takes this when its emergency password is removed: it
    /// stops being able to log in, but it is not a deactivated account.
    pub async fn revoke_credentials(&self, user_id: &UserId) -> Result<()> {
        self.all_device_ids(user_id)
            .for_each(|device_id| self.remove_device(user_id, device_id))
            .await;

        self.set_password(user_id, None)
    }

    /// Lifts a deactivation, giving the account `password`. What deactivation
    /// took besides the password (rooms, profile, admin rights) stays gone.
    pub async fn reactivate_account(&self, user_id: &UserId, password: &str) -> Result<()> {
        if !self.is_deactivated(user_id).await? {
            return Err!(Request(InvalidParam("{user_id} is not deactivated.")));
        }

        self.set_password(user_id, Some(password))?;
        self.db.userid_deactivated.remove(user_id)?;

        Ok(())
    }

    /// When and by whom an account was deactivated; `NotFound` when it is not.
    pub async fn deactivation(&self, user_id: &UserId) -> Result<Deactivation> {
        self.db.userid_deactivated.get(user_id).await.deserialized()
    }

    #[inline]
    pub async fn exists(&self, user_id: &UserId) -> bool {
        self.db.userid_password.get(user_id).await.is_ok()
    }

    /// Whether the account carries the deactivation marker; an error for an
    /// account that does not exist.
    pub async fn is_deactivated(&self, user_id: &UserId) -> Result<bool> {
        if self.db.userid_deactivated.exists(user_id).await.is_ok() {
            return Ok(true);
        }

        if !self.exists(user_id).await {
            return Err!(Request(NotFound("User does not exist.")));
        }

        Ok(false)
    }

    pub async fn is_active(&self, user_id: &UserId) -> bool {
        !self.is_deactivated(user_id).await.unwrap_or(true)
    }

    pub async fn is_active_local(&self, user_id: &UserId) -> bool {
        self.services.server_state.user_is_local(user_id) && self.is_active(user_id).await
    }

    /// Whether a local account is a guest: one with no password that was not
    /// deactivated, other than the server user.
    ///
    /// Guests are not recorded as such, so this is an inference, and it also
    /// holds for an appservice user registered without a password. Callers
    /// acting on it exempt requests made with an appservice token, as they did
    /// when guests were told apart by `is_deactivated`.
    pub async fn is_guest(&self, user_id: &UserId) -> bool {
        if *user_id == self.services.server_state.server_user
            || !self.services.server_state.user_is_local(user_id)
        {
            return false;
        }

        let passwordless = self
            .db
            .userid_password
            .get(user_id)
            .await
            .is_ok_and(|hash| hash.is_empty());

        passwordless && !self.is_deactivated(user_id).await.unwrap_or(true)
    }

    #[inline]
    pub async fn count(&self) -> usize {
        self.db.userid_password.count().await
    }

    #[allow(clippy::iter_without_into_iter, clippy::iter_not_returning_iterator)]
    pub fn iter(&self) -> impl Stream<Item = OwnedUserId> + Send + '_ {
        self.stream().map(ToOwned::to_owned)
    }

    pub fn stream(&self) -> impl Stream<Item = &UserId> + Send {
        self.db
            .userid_password
            .keys::<&str>()
            .ignore_err()
            .map(|user_id| <&UserId>::try_from(user_id).expect("valid user id in db"))
    }

    /// Every local account that is not deactivated, guests and appservice
    /// users included, and the server user excluded.
    ///
    /// Remote users are stored too, without a password, once they share a
    /// room; this used to drop them by their empty password, which dropped
    /// every passwordless local account with them.
    pub fn list_local_users(&self) -> impl Stream<Item = &UserId> + Send + '_ {
        let server_state = &self.services.server_state;

        self.stream()
            .ready_filter(move |user_id| {
                server_state.user_is_local(user_id) && *user_id != server_state.server_user
            })
            .filter_map(
                move |user_id| async move { self.is_active(user_id).await.then_some(user_id) },
            )
    }

    /// Every account carrying the deactivation marker.
    pub fn deactivated_users(&self) -> impl Stream<Item = &UserId> + Send + '_ {
        self.db
            .userid_deactivated
            .keys::<&str>()
            .ignore_err()
            .map(|user_id| <&UserId>::try_from(user_id).expect("valid user id in db"))
    }

    pub async fn password_hash(&self, user_id: &UserId) -> Result<String> {
        self.db.userid_password.get(user_id).await.deserialized()
    }

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

impl Data {
    fn txn(&self) -> Txn {
        Txn::new(&self.database.engine)
    }
}

fn increment(db: &Arc<Map>, key: &[u8]) {
    let old = db.get_blocking(key);
    let new = bytes::increment(old.ok().as_deref());
    db.insert(key, new).ok();
}
