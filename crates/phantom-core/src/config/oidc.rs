//! OpenID Connect login.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Oidc {
    /// Offer this server's own OpenID Connect provider without any upstream
    /// identity provider behind it.
    ///
    /// The OIDC server is built whenever an `identity_provider` is configured.
    /// Turning this on builds it for a server that has none, so that clients
    /// speaking next-gen auth can log in against local passwords. Requires
    /// `well_known_client`.
    ///
    /// default: false
    #[serde(default)]
    pub oidc_native_auth: bool,

    /// Requests per second one client address may make to the OIDC endpoints.
    ///
    /// This and `oidc_rc_burst_count` are a token bucket: the rate is how fast
    /// it refills, the burst is how deep it is. Leaving either at `0` turns
    /// the throttle off. It is off by default because these endpoints are
    /// reached through a browser, where a redirect chain can look like a
    /// burst; the device-code endpoints are throttled regardless.
    ///
    /// default: 0
    #[serde(default)]
    pub oidc_rc_per_second: u32,

    /// How many OIDC requests one client address may make at once.
    ///
    /// The depth of the bucket `oidc_rc_per_second` refills. Ignored while
    /// that is `0`.
    ///
    /// default: 0
    #[serde(default)]
    pub oidc_rc_burst_count: u32,

    /// Cap on the size of a response read back from an identity provider.
    ///
    /// Discovery documents, token responses and userinfo are all small; the
    /// cap is what stops a provider — or something answering in its place —
    /// from streaming until this server runs out of memory.
    ///
    /// default: 262144
    #[serde(default = "default_oidc_max_response_size")]
    pub oidc_max_response_size: usize,
}
