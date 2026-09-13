use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Membership {
    #[serde(default)]
    pub auto_accept_invites: bool,

    #[serde(default)]
    pub auto_accept_invites_direct_only: bool,

    #[serde(default)]
    pub auto_accept_invites_local_only: bool,

    #[serde(default = "default_max_make_join_attempts_per_join_attempt")]
    pub max_make_join_attempts_per_join_attempt: usize,

    #[serde(
        default = "default_deprioritize_joins_through_servers",
        with = "serde_regex"
    )]
    pub deprioritize_joins_through_servers: RegexSet,

    #[serde(default)]
    pub enforce_stripped_state_pdu_validation: bool,
}
