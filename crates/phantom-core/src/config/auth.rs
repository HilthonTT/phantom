//! Registration, login tokens, and what a new account starts with.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Auth {
    /// A shared secret required to register an account.
    ///
    /// display: sensitive
    pub registration_token: Option<String>,

    /// Path to a file holding the registration token instead of writing it
    /// into the config. The contents are read once at startup, with
    /// surrounding whitespace trimmed, and take priority over
    /// `registration_token`.
    ///
    /// example: "/etc/phantom/.reg_token"
    pub registration_token_file: Option<PathBuf>,

    /// Seconds an OpenID token stays valid for.
    ///
    /// The token proves to an integration that the bearer holds the account it
    /// names, so it wants to be long enough to be exchanged and no longer.
    ///
    /// default: 3600
    #[serde(default = "default_openid_token_ttl")]
    pub openid_token_ttl: u64,

    /// Text appended to a user's displayname when they register, after a
    /// space. Leave it empty to append nothing.
    ///
    /// example: "🏳️‍⚧️"
    ///
    /// default: ""
    #[serde(default)]
    pub new_user_displayname_suffix: String,

    /// The base URL clients reach this server on, as published in
    /// `/.well-known/matrix/client`.
    ///
    /// The OpenID Connect server derives its issuer and its endpoint URLs from
    /// this, and it is what an identity provider's callback URL is built from
    /// when one is not configured explicitly. Without it neither next-gen auth
    /// nor SSO login can be offered, because a provider has nowhere to send
    /// the user back to.
    ///
    /// example: "https://matrix.example.com/"
    pub well_known_client: Option<Url>,
}
