//! A provider's answer at its token endpoint.

use serde::Deserialize;

/// The fields read back from an upstream provider's `/token` response.
///
/// Separate from the [`Session`](super::Session) the tokens end up in, and
/// short of it by one field: the session holds the instant an access token
/// expires, while providers disagree about how to express that — some send a
/// duration, some a Unix timestamp under the same name. Only the duration is
/// read here, and the session's instant is computed from it.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    /// Token type: `bearer`, `mac`, and so on.
    pub token_type: Option<String>,

    /// The access token the provider granted.
    pub access_token: Option<String>,

    /// Seconds the access token is valid for.
    pub expires_in: Option<u64>,

    /// The token the access token is refreshed with.
    pub refresh_token: Option<String>,

    /// Seconds the refresh token is valid for.
    pub refresh_token_expires_in: Option<u64>,

    /// The scope actually granted, where the provider reports one.
    pub scope: Option<String>,

    /// The signed JWT carrying the identity claims, for a provider that
    /// really is OpenID Connect rather than bare OAuth 2.
    pub id_token: Option<String>,
}
