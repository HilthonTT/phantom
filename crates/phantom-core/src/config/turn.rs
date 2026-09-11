use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Turn {
    #[serde(default)]
    pub turn_username: String,

    #[doc = "display: sensitive"]
    #[serde(default)]
    pub turn_password: String,

    #[serde(default)]
    pub turn_uris: Vec<String>,

    #[doc = "display: sensitive"]
    #[serde(default)]
    pub turn_secret: String,

    pub turn_secret_file: Option<PathBuf>,

    #[serde(default = "default_turn_ttl")]
    pub turn_ttl: u64,
}
