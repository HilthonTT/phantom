//! The identity providers users may authorize against.
//!
//! Opens its own TOML section rather than continuing `[global]`, so it is
//! declared after every module that does continue it.

use super::prelude::*;

/// One upstream OpenID Connect identity provider.
///
/// An entry is an OAuth application this server has registered with a
/// provider: the credentials it authenticates as, and where the provider's
/// endpoints are. Nearly every URL here is optional because discovery fills it
/// in — the overrides exist for providers that publish nothing, or publish
/// something wrong.
#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(
    filename = "phantom-example.toml",
    section = "global.identity_provider.example"
)]
pub struct IdentityProvider {
    /// The brand of the service (`github`, `gitlab`, `google`) or of the
    /// software (`keycloak`, `mas`) behind this provider.
    ///
    /// A recognised brand supplies defaults, and for some it selects the
    /// workarounds that provider needs — GitHub is not an OpenID Connect
    /// provider at all and only works because it is named here. Matching is
    /// case-insensitive.
    ///
    /// Several providers may share a brand. Where exactly one carries a brand,
    /// that brand also names it, which is the short spelling a login page
    /// uses; where more than one does, only the `client_id` names it.
    ///
    /// example: "github"
    pub brand: String,

    /// The client ID the provider issued when this application was registered.
    ///
    /// This is also the provider's identifier here, so it must not change: the
    /// identities associated with a provider are keyed against it.
    pub client_id: String,

    /// The client secret the provider issued alongside the `client_id`.
    ///
    /// Unlike the ID this may be rotated freely.
    ///
    /// display: sensitive
    pub client_secret: Option<String>,

    /// Read the client secret from this file instead of writing it here.
    ///
    /// Ignored where `client_secret` is set. The file is read on each exchange
    /// rather than held in memory, must be non-empty, and is trimmed of
    /// surrounding whitespace.
    ///
    /// example: "/etc/phantom/.client_secret"
    pub client_secret_file: Option<PathBuf>,

    /// The issuer URL the provider publishes.
    ///
    /// Optional only for the brands with a known issuer; required for anything
    /// self-hosted. It must match what the provider claims in its discovery
    /// document, and it must never change, because an identity is associated
    /// to the pair of issuer and subject.
    pub issuer_url: Option<Url>,

    /// The callback URL registered with the provider.
    ///
    /// Derived from `well_known_client` when unset, as
    /// `/_matrix/client/unstable/login/sso/callback/<client_id>`. The provider
    /// must be configured with exactly the same URL.
    pub callback_url: Option<Url>,

    /// Treat this provider as the default.
    ///
    /// Which provider `/_matrix/client/v3/login/sso/redirect` — the form with
    /// no provider on the end — sends a user to. With one provider configured
    /// it is the default whether or not this is set.
    ///
    /// default: false
    #[serde(default)]
    pub default: bool,

    /// The name users see for this provider on the login page.
    ///
    /// Defaults to `brand`, which is enough unless several providers share
    /// one.
    pub name: Option<String>,

    /// The scopes to ask the provider for.
    ///
    /// Empty sends `openid email profile`, except for `brand = "mas"`, which
    /// is sent only `openid`: MAS rejects `profile`, and its userinfo returns
    /// nothing but the subject and username either way.
    ///
    /// default: []
    #[serde(default)]
    pub scope: BTreeSet<String>,

    /// Extra path components between the `issuer_url` and the `.well-known`
    /// directory discovery reads.
    ///
    /// Empty for a provider that follows the specification. A path starting
    /// with a slash replaces the issuer's path rather than extending it, and
    /// the path must end with one.
    pub base_path: Option<String>,

    /// Whether to discover this provider's endpoints.
    ///
    /// On, a provider that cannot be discovered is a configuration error and
    /// no authorization is attempted against it. Off, every endpoint below
    /// must either be set or be derivable from the issuer.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub discovery: bool,

    /// Where the provider's `openid-configuration` is, if it is not where the
    /// issuer says it should be. Rarely needed.
    pub discovery_url: Option<Url>,

    /// Overrides the discovered authorization endpoint.
    pub authorization_url: Option<Url>,

    /// Overrides the discovered token endpoint.
    pub token_url: Option<Url>,

    /// Overrides the discovered revocation endpoint.
    pub revocation_url: Option<Url>,

    /// Overrides the discovered introspection endpoint.
    pub introspection_url: Option<Url>,

    /// Overrides the discovered userinfo endpoint.
    pub userinfo_url: Option<Url>,
}

impl IdentityProvider {
    /// The provider's stable identifier, which is its client ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &str {
        self.client_id.as_str()
    }
}
