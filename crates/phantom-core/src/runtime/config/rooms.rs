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

    /// Enforce the message retention policy (`m.room.retention`) a room sets.
    /// Once an hour, every room whose policy names a `max_lifetime` has its
    /// timeline events older than that purged, local ones included. State
    /// events are always kept, since the room cannot work without them. Other
    /// servers in the room keep their own copies whatever this server does.
    ///
    /// default: false
    #[serde(default)]
    pub retention_policy_enforcement: bool,

    /// The shortest `max_lifetime` enforced, in seconds. A room whose policy
    /// asks for less is purged as if it had asked for this, so a room admin
    /// cannot make the server delete messages as soon as they arrive.
    ///
    /// default: 86400
    #[serde(default = "default_retention_min_lifetime_secs")]
    pub retention_min_lifetime_secs: u64,

    #[serde(default, with = "serde_regex")]
    pub forbidden_alias_names: RegexSet,

    #[serde(default, with = "serde_regex")]
    pub forbidden_usernames: RegexSet,
}
