use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Rooms {
    #[serde(default)]
    pub forget_forced_upon_leave: bool,

    #[serde(default = "true_fn")]
    pub allow_room_creation: bool,

    #[serde(default = "true_fn")]
    pub save_unredacted_events: bool,

    #[serde(default = "default_redaction_retention_seconds")]
    pub redaction_retention_seconds: u64,

    #[serde(default, with = "serde_regex")]
    pub forbidden_alias_names: RegexSet,

    #[serde(default, with = "serde_regex")]
    pub forbidden_usernames: RegexSet,
}
