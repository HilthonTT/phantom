mod binding;
mod canonical;
mod pending;
mod ratelimit;

pub use self::canonical::canonicalize_email;

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

use ruma::{MilliSecondsSinceUnixEpoch, OwnedDeviceId, OwnedUserId, thirdparty::Medium};
use serde::{Deserialize, Serialize};

use phantom_core::{Result, sync::MutexMap};
use phantom_database::{Database, Map};
use smallstr::SmallString;

type Ratelimiter<K> = Mutex<HashMap<K, (Instant, f64)>>;

type EmailKey = SmallString<[u8; 48]>;

pub type UiaaSessionId = SmallString<[u8; 32]>;

pub type UiaaKey = (OwnedUserId, OwnedDeviceId, UiaaSessionId);

pub struct Service {
    db: Data,
    pending_mutex: MutexMap<String, ()>,
    claim_mutex: MutexMap<String, ()>,
    ip_ratelimiter: Ratelimiter<IpAddr>,
    address_ratelimiter: Ratelimiter<EmailKey>,
}

struct Data {
    database: Arc<Database>,
    userid_email: Arc<Map>,
    email_userid: Arc<Map>,
    threepidsid_pending: Arc<Map>,
    userdevicesessionid_threepid: Arc<Map>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Binding {
    medium: Medium,
    validated_at: MilliSecondsSinceUnixEpoch,
    added_at: MilliSecondsSinceUnixEpoch,
}

/// Validated third-party identifier consumed from a pending proof.
///
/// Redemption returns the original medium and address stored with the
/// verification session. Owning both values lets the result outlive the
/// pending-row read.
#[derive(Clone, Debug)]
pub struct Association {
    /// Third-party identifier medium that was verified.
    pub medium: Medium,

    /// Address exactly as stored by the verification session.
    pub address: String,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data {
                database: args.db.clone(),
                userid_email: args.db["userid_email"].clone(),
                email_userid: args.db["email_userid"].clone(),
                threepidsid_pending: args.db["threepidsid_pending"].clone(),
                userdevicesessionid_threepid: args.db["userdevicesessionid_threepid"].clone(),
            },
            pending_mutex: MutexMap::new(),
            claim_mutex: MutexMap::new(),
            ip_ratelimiter: Mutex::new(HashMap::new()),
            address_ratelimiter: Mutex::new(HashMap::new()),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Key under which `claim_mutex` serializes work on one UIAA transaction.
///
/// `MutexMap` builds its owned key from a borrowed one, which the `UiaaKey`
/// tuple cannot provide, so the claim is flattened into a string.
fn claim_lock_key((user_id, device_id, session): &UiaaKey) -> String {
    format!("{user_id}\0{device_id}\0{session}")
}
