//! The device authorization grant, RFC 8628.
//!
//! For the clients that cannot open a browser — a TV, a terminal. The client
//! asks for a grant and is given two codes: a `device_code` it polls with, and
//! a short `user_code` the person types into a browser somewhere else. When
//! they approve it there, the next poll gets tokens.
//!
//! The `user_code` is the weak point and the reason for most of what is here.
//! It has to be short enough to read off a screen and type, so it cannot carry
//! much entropy, so §5.1 requires the guesses at it be bounded: each grant
//! counts the attempts against it and invalidates itself past a handful, and
//! the endpoint that takes one is throttled per address regardless of how the
//! rate-limit config is set.

use std::time::{Duration, SystemTime};

use phantom_core::{Err, Result, err, implement, rand};
use phantom_database::{Cbor, Deserialized};
use ruma::OwnedUserId;
use serde::{Deserialize, Serialize};

/// A pending device authorization: stored under its `device_code`, and reached
/// from the browser through the `user_code` index.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceGrant {
    pub device_code: String,

    /// Held in the normalized form that is the index key. Show it with
    /// [`format_user_code`].
    pub user_code: String,

    pub client_id: String,
    pub scope: String,
    pub status: DeviceGrantStatus,
    pub attempts: u32,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum DeviceGrantStatus {
    Pending,
    Approved {
        user_id: OwnedUserId,
        idp_id: Option<String>,
    },
    Denied,
}

/// What an approved grant hands the token endpoint.
pub struct ApprovedDeviceGrant {
    pub client_id: String,
    pub scope: String,
    pub user_id: OwnedUserId,
    pub idp_id: Option<String>,
}

/// The outcome of one poll, which the caller maps onto the RFC 8628 §3.5 error
/// codes.
pub enum DeviceGrantPoll {
    Pending,
    Approved(ApprovedDeviceGrant),
    Denied,
    Expired,
}

const DEVICE_CODE_LENGTH: usize = 64;

/// Characters in a `user_code`. Ten from a twenty-character alphabet is about
/// 43 bits, which against the always-on per-address throttle is far more than
/// a grant's lifetime allows anyone to work through, while still being short
/// enough to type.
const USER_CODE_LENGTH: usize = 10;

/// The RFC 8628 §6.1 alphabet: uppercase consonants. No vowels, so a code
/// cannot come out as a word; no digits, so nothing is confusable with `O`,
/// `I` or `S`; no lowercase, so there is nothing to hold shift for.
const USER_CODE_CHARSET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// How many times one grant may be brought to the consent screen before it
/// invalidates itself (§5.1). Generous enough for a reloaded page.
const MAX_VERIFY_ATTEMPTS: u32 = 10;

/// How long the person at the browser has to approve.
pub const DEVICE_GRANT_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// How long the client is told to wait between polls (§3.2).
pub const DEVICE_GRANT_INTERVAL_SECS: u64 = 5;

/// Opens a device authorization grant and both of its codes.
#[implement(super::Server)]
pub fn create_device_grant(&self, client_id: &str, scope: &str) -> Result<DeviceGrant> {
    let now = SystemTime::now();
    let device_code = rand::string(DEVICE_CODE_LENGTH);
    let user_code = rand::string_from(USER_CODE_CHARSET, USER_CODE_LENGTH);

    let grant = DeviceGrant {
        device_code: device_code.clone(),
        user_code: user_code.clone(),
        client_id: client_id.to_owned(),
        scope: scope.to_owned(),
        status: DeviceGrantStatus::Pending,
        attempts: 0,
        created_at: now,
        expires_at: now.checked_add(DEVICE_GRANT_LIFETIME).unwrap_or(now),
    };

    self.db
        .oidcdevicecode_devicegrant
        .raw_put(&device_code, Cbor(&grant))?;

    self.db
        .oidcusercode_devicecode
        .raw_put(&user_code, Cbor(&device_code))?;

    Ok(grant)
}

/// Looks a grant up for the consent screen, counting the attempt.
///
/// Past [`MAX_VERIFY_ATTEMPTS`] the grant invalidates itself: at that point the
/// codes being tried are guesses, and the person the grant belongs to can ask
/// for a new one.
#[implement(super::Server)]
pub async fn verify_device_grant(&self, user_code: &str) -> Result<DeviceGrant> {
    let device_code = self.resolve_device_code(user_code).await?;
    let _lock = self.device_locks.lock(&device_code).await;

    let mut grant = self.get_device_grant(&device_code).await?;

    if SystemTime::now() > grant.expires_at {
        self.remove_device_grant(&grant.device_code, &grant.user_code)?;

        return Err!(Request(NotFound("The device authorization has expired")));
    }

    if !matches!(grant.status, DeviceGrantStatus::Pending) {
        return Err!(Request(Forbidden(
            "The device authorization was already resolved"
        )));
    }

    grant.attempts = grant.attempts.saturating_add(1);

    if grant.attempts > MAX_VERIFY_ATTEMPTS {
        self.remove_device_grant(&grant.device_code, &grant.user_code)?;

        return Err!(Request(Forbidden("Too many attempts; request a new code")));
    }

    self.db
        .oidcdevicecode_devicegrant
        .raw_put(&grant.device_code, Cbor(&grant))?;

    Ok(grant)
}

/// Approves a grant on behalf of the user at the browser.
#[implement(super::Server)]
pub async fn approve_device_grant(
    &self,
    user_code: &str,
    user_id: OwnedUserId,
    idp_id: Option<String>,
) -> Result {
    self.set_device_grant_status(user_code, DeviceGrantStatus::Approved { user_id, idp_id })
        .await
}

/// Denies a grant on behalf of the user at the browser.
#[implement(super::Server)]
pub async fn deny_device_grant(&self, user_code: &str) -> Result {
    self.set_device_grant_status(user_code, DeviceGrantStatus::Denied)
        .await
}

/// Polls a grant by its `device_code` (§3.4).
///
/// A terminal outcome consumes the grant; a pending one is left for the next
/// poll. The whole read-check-consume is under the grant's lock, so two polls
/// arriving together cannot both come back approved and mint two devices from
/// one authorization.
#[implement(super::Server)]
pub async fn poll_device_grant(
    &self,
    device_code: &str,
    client_id: &str,
) -> Result<DeviceGrantPoll> {
    let _lock = self.device_locks.lock(device_code).await;

    let grant = self.get_device_grant(device_code).await?;

    if grant.client_id != client_id {
        return Err!(Request(Forbidden("client_id mismatch")));
    }

    if SystemTime::now() > grant.expires_at {
        self.remove_device_grant(&grant.device_code, &grant.user_code)?;

        return Ok(DeviceGrantPoll::Expired);
    }

    match grant.status {
        DeviceGrantStatus::Pending => Ok(DeviceGrantPoll::Pending),
        DeviceGrantStatus::Denied => {
            self.remove_device_grant(&grant.device_code, &grant.user_code)?;

            Ok(DeviceGrantPoll::Denied)
        }
        DeviceGrantStatus::Approved { user_id, idp_id } => {
            self.remove_device_grant(&grant.device_code, &grant.user_code)?;

            Ok(DeviceGrantPoll::Approved(ApprovedDeviceGrant {
                client_id: grant.client_id,
                scope: grant.scope,
                user_id,
                idp_id,
            }))
        }
    }
}

/// The `device_code` a user-entered code resolves to, through the index.
#[implement(super::Server)]
async fn resolve_device_code(&self, user_code: &str) -> Result<String> {
    let user_code = normalize_user_code(user_code);

    self.db
        .oidcusercode_devicecode
        .get(&user_code)
        .await
        .deserialized::<Cbor<String>>()
        .map(|Cbor(device_code)| device_code)
        .map_err(|_| err!(Request(NotFound("Unknown or expired user code"))))
}

#[implement(super::Server)]
async fn get_device_grant(&self, device_code: &str) -> Result<DeviceGrant> {
    self.db
        .oidcdevicecode_devicegrant
        .get(device_code)
        .await
        .deserialized::<Cbor<DeviceGrant>>()
        .map(|Cbor(grant)| grant)
        .map_err(|_| err!(Request(Forbidden("Invalid or expired device code"))))
}

#[implement(super::Server)]
async fn set_device_grant_status(&self, user_code: &str, status: DeviceGrantStatus) -> Result {
    let device_code = self.resolve_device_code(user_code).await?;
    let _lock = self.device_locks.lock(&device_code).await;

    let mut grant = self.get_device_grant(&device_code).await?;

    if SystemTime::now() > grant.expires_at {
        self.remove_device_grant(&grant.device_code, &grant.user_code)?;

        return Err!(Request(NotFound("The device authorization has expired")));
    }

    if !matches!(grant.status, DeviceGrantStatus::Pending) {
        return Err!(Request(Forbidden(
            "The device authorization was already resolved"
        )));
    }

    grant.status = status;

    self.db
        .oidcdevicecode_devicegrant
        .raw_put(&grant.device_code, Cbor(&grant))
}

#[implement(super::Server)]
fn remove_device_grant(&self, device_code: &str, user_code: &str) -> Result {
    self.db.oidcdevicecode_devicegrant.remove(device_code)?;

    self.db.oidcusercode_devicecode.remove(user_code)
}

/// Folds what a person typed back to the stored form.
///
/// Case, hyphens and spaces are all things a user code is read and written
/// with, and none of them are in the alphabet, so dropping everything that is
/// not in it is exactly the normalization §6.1 asks for.
fn normalize_user_code(input: &str) -> String {
    input
        .bytes()
        .map(|b| b.to_ascii_uppercase())
        .filter(|b| USER_CODE_CHARSET.contains(b))
        .map(char::from)
        .collect()
}

/// Renders a stored user code for a person to read, split by one hyphen.
#[must_use]
pub fn format_user_code(code: &str) -> String {
    code.split_at_checked(code.len() / 2)
        .filter(|(head, tail)| !head.is_empty() && !tail.is_empty())
        .map(|(head, tail)| format!("{head}-{tail}"))
        .unwrap_or_else(|| code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{USER_CODE_CHARSET, USER_CODE_LENGTH, format_user_code, normalize_user_code};

    #[test]
    fn format_then_normalize_round_trips() {
        let code = "BCDFGHJK";

        assert_eq!(normalize_user_code(&format_user_code(code)), code);
    }

    #[test]
    fn normalize_strips_separators_and_uppercases() {
        assert_eq!(normalize_user_code("bcdf-ghjk"), "BCDFGHJK");
        assert_eq!(normalize_user_code(" bc df ghjk "), "BCDFGHJK");
    }

    #[test]
    fn normalize_drops_out_of_charset_characters() {
        assert_eq!(normalize_user_code("B0C1DAEF"), "BCDF");
    }

    #[test]
    fn format_inserts_a_single_separator() {
        assert_eq!(format_user_code("BCDFGHJK"), "BCDF-GHJK");
    }

    #[test]
    fn charset_is_base20_without_vowels_or_digits() {
        assert_eq!(USER_CODE_CHARSET.len(), 20);
        assert_eq!(USER_CODE_LENGTH, 10);

        // RFC 8628 drops the vowels and Y and keeps every other consonant.
        for excluded in b"AEIOUY0123456789" {
            assert!(!USER_CODE_CHARSET.contains(excluded));
        }
    }
}
