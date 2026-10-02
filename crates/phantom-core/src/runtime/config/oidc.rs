use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Oidc {
    #[serde(default)]
    pub oidc_native_auth: bool,

    /// Require PKCE (S256) on every authorization request.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub oidc_require_pkce: bool,

    /// Reject, rather than drop, an unrecognised scope.
    ///
    /// default: false
    #[serde(default)]
    pub oidc_strict_scope: bool,

    /// Refuse a token grant whose scope names no MSC2967 device.
    ///
    /// default: false
    #[serde(default)]
    pub oidc_require_device_scope: bool,

    /// RFC 7591 initial access token; when set, dynamic client registration
    /// requires it as a bearer token.
    ///
    /// default:
    #[doc = "display: sensitive"]
    #[serde(default)]
    pub oidc_registration_access_token: String,

    /// Require registering clients to supply `client_uri`, which every other
    /// URI must then share a host with.
    ///
    /// default: false
    #[serde(default)]
    pub oidc_registration_require_client_uri: bool,

    /// Hosts, or private-use schemes, every registered `redirect_uri` must
    /// name. Empty allows any. Listed targets also skip the approval prompt.
    ///
    /// default: []
    #[serde(default)]
    pub oidc_registration_allowed_redirect_hosts: Vec<String>,

    /// Ask the user to approve each client before the authorization code is
    /// released.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub oidc_require_client_approval: bool,

    #[serde(default)]
    pub oidc_rc_per_second: u32,

    #[serde(default)]
    pub oidc_rc_burst_count: u32,

    #[serde(default = "default_oidc_max_response_size")]
    pub oidc_max_response_size: usize,
}
