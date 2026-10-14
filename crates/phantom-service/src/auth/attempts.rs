//! How often a client may try to prove who it is: logins per address, wrong
//! passwords per account, and registrations per address.
//!
//! Each is a token bucket from [`crate::ratelimit`], sized by the `login_rc_*`
//! and `registration_rc_*` options. The per-address limits count what a
//! client does; the per-account one counts what is done to an account, which
//! is what still holds when the guesses come from many addresses. Callers
//! exempt appservices, and the admin API never reaches here: it has no login
//! of its own.

use std::{net::IpAddr, sync::Arc};

use phantom_core::{Result, runtime::server::Server};
use ruma::{DeviceId, OwnedDeviceId, OwnedUserId, UserId};

use crate::ratelimit::{Limit, Ratelimiter, check, peek, refund};

pub struct Service {
    server: Arc<Server>,
    tables: Tables,
}

#[derive(Default)]
struct Tables {
    login: Ratelimiter<IpAddr>,
    failed: Ratelimiter<OwnedUserId>,
    session_failed: Ratelimiter<(OwnedUserId, OwnedDeviceId)>,
    registration: Ratelimiter<IpAddr>,
}

/// A registration's token, taken from its address's allowance before the
/// account exists. Dropped without [`commit`](Self::commit) — an interactive
/// auth round trip, an error, a request the client gave up on — it is given
/// back, so only registrations that create an account count. Taking it up
/// front is what stops concurrent registrations all passing one check.
#[must_use]
pub struct RegistrationToken<'a> {
    service: &'a Service,
    client: IpAddr,
    committed: bool,
}

impl RegistrationToken<'_> {
    /// Keeps the token spent: the account exists.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for RegistrationToken<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.service
                .tables
                .refund_registration(self.client, self.service.registration_limit());
        }
    }
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            server: args.server.clone(),
            tables: Tables::default(),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Spends one of `client`'s login attempts, refusing once they run out.
    pub fn check_login(&self, client: IpAddr) -> Result {
        let auth = &self.server.config.auth;
        let limit = limit(
            auth.login_rc_per_second,
            auth.login_rc_burst_count,
            "Too many login attempts from this address.",
        );

        self.tables.check_login(client, limit)
    }

    /// Refuses a password attempt against `user_id` while its wrong-password
    /// allowance is spent. Spends nothing; [`Self::record_failed`] does.
    pub fn check_password(&self, user_id: &UserId) -> Result {
        self.tables.check_password(user_id, self.failed_limit())
    }

    /// Charges a wrong password to `user_id`'s allowance.
    pub fn record_failed(&self, user_id: &UserId) {
        self.tables.record_failed(user_id, self.failed_limit());
    }

    /// Refuses a password attempt from an authenticated session while the
    /// device's wrong-password allowance is spent. Interactive auth is keyed
    /// by device rather than account: the access token is already proven, so
    /// nobody else can spend the owner's allowance and lock them out of
    /// changing their own password. Spends nothing;
    /// [`Self::record_session_failed`] does.
    pub fn check_session_password(&self, user_id: &UserId, device_id: &DeviceId) -> Result {
        self.tables
            .check_session_password(user_id, device_id, self.failed_limit())
    }

    /// Charges a wrong password to the device's allowance.
    pub fn record_session_failed(&self, user_id: &UserId, device_id: &DeviceId) {
        self.tables
            .record_session_failed(user_id, device_id, self.failed_limit());
    }

    /// Takes a token from `client`'s registration allowance, refusing once it
    /// is spent. The token is given back unless the registration commits it.
    pub fn reserve_registration(&self, client: IpAddr) -> Result<RegistrationToken<'_>> {
        self.tables
            .reserve_registration(client, self.registration_limit())?;

        Ok(RegistrationToken {
            service: self,
            client,
            committed: false,
        })
    }

    fn failed_limit(&self) -> Limit {
        let auth = &self.server.config.auth;

        limit(
            auth.login_rc_failed_per_second,
            auth.login_rc_failed_burst_count,
            "Too many failed login attempts for this account.",
        )
    }

    fn registration_limit(&self) -> Limit {
        let auth = &self.server.config.auth;

        limit(
            auth.registration_rc_per_second,
            auth.registration_rc_burst_count,
            "Too many registrations from this address.",
        )
    }
}

impl Tables {
    fn check_login(&self, client: IpAddr, limit: Limit) -> Result {
        if limit.is_disabled() {
            return Ok(());
        }

        check(
            &self.login,
            &client.to_canonical(),
            || client.to_canonical(),
            limit,
        )
    }

    fn check_password(&self, user_id: &UserId, limit: Limit) -> Result {
        if limit.is_disabled() {
            return Ok(());
        }

        peek(&self.failed, user_id, limit)
    }

    fn record_failed(&self, user_id: &UserId, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        // Refused only when the allowance is already spent, which the check
        // before the attempt has reported; the failure is still recorded.
        check(&self.failed, user_id, || user_id.to_owned(), limit).ok();
    }

    fn check_session_password(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        limit: Limit,
    ) -> Result {
        if limit.is_disabled() {
            return Ok(());
        }

        peek(
            &self.session_failed,
            &(user_id.to_owned(), device_id.to_owned()),
            limit,
        )
    }

    fn record_session_failed(&self, user_id: &UserId, device_id: &DeviceId, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        let key = (user_id.to_owned(), device_id.to_owned());
        check(&self.session_failed, &key, || key.clone(), limit).ok();
    }

    fn reserve_registration(&self, client: IpAddr, limit: Limit) -> Result {
        if limit.is_disabled() {
            return Ok(());
        }

        let client = client.to_canonical();
        check(&self.registration, &client, || client, limit)
    }

    fn refund_registration(&self, client: IpAddr, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        refund(&self.registration, &client.to_canonical(), limit);
    }
}

fn limit(rate: f64, burst: u32, message: &'static str) -> Limit {
    Limit {
        rate,
        burst: f64::from(burst),
        message,
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use ruma::{device_id, user_id};

    use super::{Limit, Tables, limit};

    const STRICT: Limit = Limit {
        rate: 0.001,
        burst: 2.0,
        message: "Too many.",
    };

    const CLIENT: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

    #[test]
    fn logins_from_one_address_run_out() {
        let tables = Tables::default();

        assert!(tables.check_login(CLIENT, STRICT).is_ok());
        assert!(tables.check_login(CLIENT, STRICT).is_ok());
        assert!(tables.check_login(CLIENT, STRICT).is_err());

        let other = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
        assert!(
            tables.check_login(other, STRICT).is_ok(),
            "another address has its own allowance"
        );
    }

    #[test]
    fn a_mapped_address_shares_its_ipv4_allowance() {
        let tables = Tables::default();
        let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped());

        assert!(tables.check_login(CLIENT, STRICT).is_ok());
        assert!(tables.check_login(mapped, STRICT).is_ok());
        assert!(
            tables.check_login(CLIENT, STRICT).is_err(),
            "the IPv4-mapped form is the same client"
        );
        assert!(
            tables
                .check_login(IpAddr::V6(Ipv6Addr::LOCALHOST), STRICT)
                .is_ok()
        );
    }

    #[test]
    fn only_failures_count_against_an_account() {
        let tables = Tables::default();
        let alice = user_id!("@alice:example.com");

        // Checking alone, as every successful login does, spends nothing.
        for _ in 0..10 {
            assert!(tables.check_password(alice, STRICT).is_ok());
        }

        tables.record_failed(alice, STRICT);
        tables.record_failed(alice, STRICT);

        assert!(
            tables.check_password(alice, STRICT).is_err(),
            "two wrong passwords spend a burst of two"
        );
        assert!(
            tables
                .check_password(user_id!("@bob:example.com"), STRICT)
                .is_ok()
        );
    }

    #[test]
    fn concurrent_registrations_each_hold_a_token() {
        let tables = Tables::default();

        // Two in flight at once take the burst of two; a third is refused
        // before either has finished.
        assert!(tables.reserve_registration(CLIENT, STRICT).is_ok());
        assert!(tables.reserve_registration(CLIENT, STRICT).is_ok());
        assert!(tables.reserve_registration(CLIENT, STRICT).is_err());
    }

    #[test]
    fn a_registration_that_creates_nothing_gives_its_token_back() {
        let tables = Tables::default();

        for _ in 0..10 {
            assert!(tables.reserve_registration(CLIENT, STRICT).is_ok());
            tables.refund_registration(CLIENT, STRICT);
        }

        assert!(tables.reserve_registration(CLIENT, STRICT).is_ok());
        assert!(tables.reserve_registration(CLIENT, STRICT).is_ok());
        assert!(tables.reserve_registration(CLIENT, STRICT).is_err());
    }

    #[test]
    fn a_session_spends_its_own_allowance() {
        let tables = Tables::default();
        let alice = user_id!("@alice:example.com");
        let (phone, laptop) = (device_id!("PHONE"), device_id!("LAPTOP"));

        tables.record_session_failed(alice, phone, STRICT);
        tables.record_session_failed(alice, phone, STRICT);

        assert!(tables.check_session_password(alice, phone, STRICT).is_err());
        assert!(
            tables.check_session_password(alice, laptop, STRICT).is_ok(),
            "another session of the same account is not locked out"
        );
        assert!(
            tables.check_password(alice, STRICT).is_ok(),
            "nor is the account's login"
        );
    }

    #[test]
    fn a_zero_rate_turns_a_limit_off() {
        let tables = Tables::default();
        let off = limit(0.0, 10, "Too many.");

        for _ in 0..100 {
            assert!(tables.check_login(CLIENT, off).is_ok());
            tables.record_failed(user_id!("@alice:example.com"), off);
        }

        assert!(
            tables
                .check_password(user_id!("@alice:example.com"), off)
                .is_ok()
        );
    }
}
