use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Admin {
    #[doc = "display: sensitive"]
    pub emergency_password: Option<String>,

    #[serde(default)]
    pub admin_execute: Vec<String>,

    #[serde(default)]
    pub admin_signal_execute: Vec<String>,

    #[serde(default)]
    pub admin_execute_errors_ignore: bool,

    #[serde(default = "true_fn")]
    pub admin_escape_commands: bool,

    #[serde(default = "true_fn")]
    pub config_reload_signal: bool,
}
