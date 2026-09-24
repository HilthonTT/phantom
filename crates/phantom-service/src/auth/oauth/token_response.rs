use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub token_type: Option<String>,

    pub access_token: Option<String>,

    pub expires_in: Option<u64>,

    pub refresh_token: Option<String>,

    pub refresh_token_expires_in: Option<u64>,

    pub scope: Option<String>,

    pub id_token: Option<String>,
}
