use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Updates {
    #[serde(default)]
    pub allow_check_for_updates: bool,

    #[serde(default = "default_check_for_updates_url")]
    pub check_for_updates_url: String,

    #[serde(default = "default_check_for_updates_interval_s")]
    pub check_for_updates_interval_s: u64,
}
