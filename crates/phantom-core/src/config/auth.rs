use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Auth {
    #[doc = "display: sensitive"]
    pub registration_token: Option<String>,

    pub registration_token_file: Option<PathBuf>,

    #[serde(default = "default_openid_token_ttl")]
    pub openid_token_ttl: u64,

    #[serde(default)]
    pub new_user_displayname_suffix: String,

    pub well_known_client: Option<Url>,
}
