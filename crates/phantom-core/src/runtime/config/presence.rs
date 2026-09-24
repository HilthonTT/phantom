use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Presence {
    #[serde(default = "default_presence_idle_timeout_s")]
    pub presence_idle_timeout_s: u64,

    #[serde(default = "default_presence_offline_timeout_s")]
    pub presence_offline_timeout_s: u64,

    #[serde(default = "true_fn")]
    pub presence_timeout_remote_users: bool,

    #[serde(default = "true_fn")]
    pub allow_outgoing_presence: bool,

    #[serde(default = "true_fn")]
    pub allow_outgoing_read_receipts: bool,

    #[serde(default = "true_fn")]
    pub allow_outgoing_typing: bool,
}
