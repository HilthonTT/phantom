use std::net::IpAddr;

use phantom_core::{Result, implement};

use super::EmailKey;
use crate::ratelimit::{Limit, check};

/// Verification sends are slow and cost an email, so the bucket is small: a
/// burst of five, refilling one every five seconds.
const VERIFICATION: Limit = Limit {
    rate: 0.2,
    burst: 5.0,
    message: "Too many verification requests.",
};

#[implement(super::Service)]
pub fn check_address_rate_limit(&self, address: &str) -> Result {
    check(
        &self.address_ratelimiter,
        address,
        || EmailKey::from(address),
        VERIFICATION,
    )
}

#[implement(super::Service)]
pub fn check_ip_rate_limit(&self, client: IpAddr) -> Result {
    check(&self.ip_ratelimiter, &client, || client, VERIFICATION)
}
