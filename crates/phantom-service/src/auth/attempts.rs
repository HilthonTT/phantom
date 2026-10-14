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
use ruma::{OwnedUserId, UserId};

use crate::ratelimit::{Limit, Ratelimiter, check, peek};

pub struct Service {
    server: Arc<Server>,
    tables: Tables,
}

#[derive(Default)]
struct Tables {
    login: Ratelimiter<IpAddr>,
    failed: Ratelimiter<OwnedUserId>,
    registration: Ratelimiter<IpAddr>,
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

    /// Refuses a registration from `client` while its allowance is spent.
    /// Spends nothing; [`Self::record_registration`] does, once an account
    /// exists.
    pub fn check_registration(&self, client: IpAddr) -> Result {
        self.tables
            .check_registration(client, self.registration_limit())
    }

    /// Charges a created account to `client`'s registration allowance.
    pub fn record_registration(&self, client: IpAddr) {
        self.tables
            .record_registration(client, self.registration_limit());
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

    fn check_registration(&self, client: IpAddr, limit: Limit) -> Result {
        if limit.is_disabled() {
            return Ok(());
        }

        peek(&self.registration, &client.to_canonical(), limit)
    }

    fn record_registration(&self, client: IpAddr, limit: Limit) {
        if limit.is_disabled() {
            return;
        }

        let client = client.to_canonical();
        check(&self.registration, &client, || client, limit).ok();
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

    use ruma::user_id;

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
    fn registrations_count_once_an_account_exists() {
        let tables = Tables::default();

        assert!(tables.check_registration(CLIENT, STRICT).is_ok());
        assert!(tables.check_registration(CLIENT, STRICT).is_ok());

        tables.record_registration(CLIENT, STRICT);
        tables.record_registration(CLIENT, STRICT);

        assert!(tables.check_registration(CLIENT, STRICT).is_err());
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
