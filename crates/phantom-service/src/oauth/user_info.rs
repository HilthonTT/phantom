//! What a provider says about the person who just authorized.
//!
//! Only the claims this server has a use for. The shape is deliberately
//! forgiving — every field but the subject is optional, and providers disagree
//! about which of them they send — because the alternative is refusing a login
//! over a claim nobody needed.

use serde::{Deserialize, Serialize};

/// The userinfo claims this server reads.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UserInfo {
    /// The provider's own identifier for this person: usually a number, but a
    /// login name at some. Paired with the issuer it is what identifies an
    /// identity across the whole of the internet, and that pair is what
    /// `oauthuniqid_oauthid` is keyed on.
    ///
    /// Considered for a Matrix localpart only when none of the fields below
    /// are set. The `login` alias is GitHub, which sends no `sub`.
    #[serde(alias = "login")]
    pub sub: String,

    /// The login name to prefer, where the provider offers one.
    pub preferred_username: Option<String>,

    /// The login name, where the provider sends it under this name instead.
    pub username: Option<String>,

    /// The login name to fall back to when nothing is preferred.
    pub nickname: Option<String>,

    /// Full name.
    pub name: Option<String>,

    /// First name.
    pub given_name: Option<String>,

    /// Last name.
    pub family_name: Option<String>,

    /// Email address, where the `email` scope was granted.
    pub email: Option<String>,

    /// Profile picture, as GitHub and GitLab name it.
    pub avatar_url: Option<String>,

    /// Profile picture, as Google names it.
    pub picture: Option<String>,
}
