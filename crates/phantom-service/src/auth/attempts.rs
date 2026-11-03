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

use crate::ratelimit::{Limit, Ratelimiter, check, refund};

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

/// A password attempt's token, taken from the account's (or session's)
/// wrong-password allowance before the hash is checked. Dropped without
/// [`failed`](Self::failed) it is given back, so only wrong passwords count.
/// Taking it before the slow hash is what stops a burst of concurrent
/// guesses all passing one check before any failure is recorded.
#[must_use]
pub struct PasswordAttempt<'a> {
    service: &'a Service,
    key: AttemptKey,
    failed: bool,
}

enum AttemptKey {
    Account(OwnedUserId),
    Session(OwnedUserId, OwnedDeviceId),
}

impl PasswordAttempt<'_> {
    /// Keeps the token spent: the password was wrong.
    pub fn failed(mut self) {
        self.failed = true;
    }
}

impl Drop for PasswordAttempt<'_> {
    fn drop(&mut self) {
        if self.failed {
            return;
        }

        let limit = self.service.failed_limit();
        match &self.key {
            AttemptKey::Account(user_id) => self.service.tables.refund_password(user_id, limit),
            AttemptKey::Session(user_id, device_id) => self
                .service
                .tables
                .refund_session_password(user_id, device_id, limit),
        }
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

    /// Takes a token from `user_id`'s wrong-password allowance, refusing once
    /// it is spent. The token is given back unless the attempt is marked
    /// [`failed`](PasswordAttempt::failed).
    pub fn check_password(&self, user_id: &UserId) -> Result<PasswordAttempt<'_>> {
        self.tables.check_password(user_id, self.failed_limit())?;

        Ok(PasswordAttempt {
            service: self,
            key: AttemptKey::Account(user_id.to_owned()),
            failed: false,
        })
    }

    /// Refuses a password attempt from an authenticated session while the
    /// device's wrong-password allowance is spent. Interactive auth is keyed
    /// by device rather than account: the access token is already proven, so
    /// nobody else can spend the owner's allowance and lock them out of
    /// changing their own password. Like [`Self::check_password`], the token
    /// is given back unless the attempt is marked failed.
    pub fn check_session_password(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> Result<PasswordAttempt<'_>> {
        self.tables
            .check_session_password(user_id, device_id, self.failed_limit())?;

        Ok(PasswordAttempt {
            service: self,
            key: AttemptKey::Session(user_id.to_owned(), device_id.to_owned()),
            failed: false,
        })
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

        check(&self.failed, user_id, || user_id.to_owned(), limit)
    }

    fn refund_password(&self, user_id: &UserId, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        refund(&self.failed, user_id, limit);
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

        let key = (user_id.to_owned(), device_id.to_owned());
        check(&self.session_failed, &key, || key.clone(), limit)
    }

    fn refund_session_password(&self, user_id: &UserId, device_id: &DeviceId, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        refund(
            &self.session_failed,
            &(user_id.to_owned(), device_id.to_owned()),
            limit,
        );
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

        // A successful login gives its token back, so it spends nothing.
        for _ in 0..10 {
            assert!(tables.check_password(alice, STRICT).is_ok());
            tables.refund_password(alice, STRICT);
        }

        // Two wrong passwords keep their tokens.
        assert!(tables.check_password(alice, STRICT).is_ok());
        assert!(tables.check_password(alice, STRICT).is_ok());

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
    fn concurrent_guesses_each_hold_a_token() {
        let tables = Tables::default();
        let alice = user_id!("@alice:example.com");

        // Two guesses in flight take the burst of two; a third is refused
        // before either hash check has finished.
        assert!(tables.check_password(alice, STRICT).is_ok());
        assert!(tables.check_password(alice, STRICT).is_ok());
        assert!(tables.check_password(alice, STRICT).is_err());
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

        assert!(tables.check_session_password(alice, phone, STRICT).is_ok());
        assert!(tables.check_session_password(alice, phone, STRICT).is_ok());

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
            assert!(
                tables
                    .check_password(user_id!("@alice:example.com"), off)
                    .is_ok()
            );
        }

        assert!(
            tables
                .check_password(user_id!("@alice:example.com"), off)
                .is_ok()
        );
    }
}
