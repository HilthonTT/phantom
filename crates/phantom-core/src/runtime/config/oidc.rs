use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Oidc {
    #[serde(default)]
    pub oidc_native_auth: bool,

    #[serde(default)]
    pub oidc_rc_per_second: u32,

    #[serde(default)]
    pub oidc_rc_burst_count: u32,

    #[serde(default = "default_oidc_max_response_size")]
    pub oidc_max_response_size: usize,
}
