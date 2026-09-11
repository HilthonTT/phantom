use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UserInfo {
    #[serde(alias = "login")]
    pub sub: String,

    pub preferred_username: Option<String>,

    pub username: Option<String>,

    pub nickname: Option<String>,

    pub name: Option<String>,

    pub given_name: Option<String>,

    pub family_name: Option<String>,

    pub email: Option<String>,

    pub avatar_url: Option<String>,

    pub picture: Option<String>,
}
