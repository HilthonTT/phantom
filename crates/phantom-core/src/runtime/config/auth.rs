use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Auth {
    #[doc = "display: sensitive"]
    pub registration_token: Option<String>,

    pub registration_token_file: Option<PathBuf>,

    /// Open local registration (native OIDC sign-up included).
    ///
    /// default: false
    #[serde(default)]
    pub allow_registration: bool,

    /// Seconds an expiring access token stays valid before the client must
    /// refresh it.
    ///
    /// default: 604800
    #[serde(default = "default_access_token_ttl")]
    pub access_token_ttl: u64,

    /// Seconds a refresh token stays valid; `0` disables refresh-token expiry.
    ///
    /// default: 0
    #[serde(default)]
    pub refresh_token_ttl: u64,

    /// Slide the refresh-token deadline on every rotation (idle timeout)
    /// rather than carrying the first deadline forward (absolute lifetime).
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub refresh_token_idle_only: bool,

    /// Delete the whole device, rather than just its refresh token, when an
    /// expired refresh token is presented.
    ///
    /// default: false
    #[serde(default)]
    pub refresh_token_hard_logout: bool,

    /// Seconds after rotation in which presenting the previous refresh token
    /// counts as a benign double-submit; `0` treats every reuse as a replay.
    ///
    /// default: 15
    #[serde(default = "default_refresh_token_reuse_grace")]
    pub refresh_token_reuse_grace: u64,

    /// Delete the device when a spent refresh token is replayed outside the
    /// grace window.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub refresh_token_reuse_revoke: bool,

    #[serde(default = "default_openid_token_ttl")]
    pub openid_token_ttl: u64,

    #[serde(default)]
    pub new_user_displayname_suffix: String,

    pub well_known_client: Option<Url>,

    pub well_known_server: Option<OwnedServerName>,
}
