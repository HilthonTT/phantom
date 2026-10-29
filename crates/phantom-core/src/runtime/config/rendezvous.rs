use super::prelude::*;

/// Rendezvous sessions for signing in on a new device by QR code: MSC4108,
/// and its successor MSC4388. Two devices swap encrypted messages through a
/// short-lived session; the login itself goes through the OIDC server's
/// device authorization grant, so `well_known_client` must be set.
#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Rendezvous {
    /// Serve the rendezvous API and advertise QR-code login.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rendezvous_enabled: bool,

    /// Largest message a session holds, in bytes.
    ///
    /// default: 4096
    #[serde(default = "default_rendezvous_session_max_bytes")]
    pub rendezvous_session_max_bytes: usize,

    /// Seconds a session lives after its last write.
    ///
    /// default: 600
    #[serde(default = "default_rendezvous_session_ttl")]
    pub rendezvous_session_ttl: u64,

    /// Sessions held at once; creating one more drops the oldest. Sessions
    /// live in memory and do not survive a restart.
    ///
    /// default: 100
    #[serde(default = "default_rendezvous_max_sessions")]
    pub rendezvous_max_sessions: usize,

    /// Only a signed-in device may create a session, so the QR code is shown
    /// by the existing device and scanned by the new one. When false, anyone
    /// may create sessions, up to the rate limit below.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub rendezvous_authenticated_only: bool,

    /// Rendezvous requests one client address may make per second.
    ///
    /// default: 10
    #[serde(default = "default_rendezvous_rc_per_second")]
    pub rendezvous_rc_per_second: u32,

    /// Rendezvous requests one client address may make in a burst before
    /// `rendezvous_rc_per_second` applies.
    ///
    /// default: 20
    #[serde(default = "default_rendezvous_rc_burst_count")]
    pub rendezvous_rc_burst_count: u32,
}
