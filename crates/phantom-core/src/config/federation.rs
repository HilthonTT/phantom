use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Federation {
    #[serde(default)]
    pub allow_public_room_directory_over_federation: bool,

    #[serde(default)]
    pub allow_device_name_federation: bool,

    #[serde(default = "default_trusted_servers")]
    pub trusted_servers: Vec<OwnedServerName>,

    #[serde(default)]
    pub query_trusted_key_servers_first: bool,

    #[serde(default = "true_fn")]
    pub query_trusted_key_servers_first_on_join: bool,

    #[serde(default)]
    pub only_query_trusted_key_servers: bool,

    #[serde(default = "default_trusted_server_batch_size")]
    pub trusted_server_batch_size: usize,

    #[serde(default = "true_fn")]
    pub allow_federation: bool,

    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_server_names: RegexSet,

    #[serde(default = "default_federation_prev_event_budget_s")]
    pub federation_prev_event_budget_s: u64,

    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_room_directory_server_names: RegexSet,

    #[serde(default, with = "serde_regex")]
    pub forbidden_remote_media_server_names: RegexSet,

    #[serde(default)]
    pub federation_loopback: bool,

    #[serde(default = "default_fetch_fanout_max_width")]
    pub fetch_fanout_max_width: usize,

    #[serde(default = "default_fetch_fanout_rounds")]
    pub fetch_fanout_rounds: usize,
}
