//! Rooms: what may be created, and what is kept.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Rooms {
    /// Makes leaving a room also forget it, rather than leaving it in the
    /// user's `leave` section of sync until they forget it themselves.
    ///
    /// Banned and admin-disabled rooms are forgotten on leave either way.
    ///
    /// default: false
    #[serde(default)]
    pub forget_forced_upon_leave: bool,

    /// Allow ordinary users to create rooms. Admins and appservices may
    /// always create them regardless of this.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_room_creation: bool,

    /// Keep the original of an event before a redaction strips it.
    ///
    /// A redaction removes content the spec does not require the event to
    /// keep, and the copy retained here is what a moderator reviewing a report
    /// reads afterwards. Leaving this off makes a redaction final on arrival.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub save_unredacted_events: bool,

    /// Seconds an original retained by `save_unredacted_events` is kept before
    /// it is swept.
    ///
    /// The default is 60 days. Zero keeps the originals indefinitely, which
    /// grows without bound — set it only where something else prunes them.
    ///
    /// default: 5184000
    #[serde(default = "default_redaction_retention_seconds")]
    pub redaction_retention_seconds: u64,

    /// Room aliases and room IDs that may not be created, as regular
    /// expressions. A plain word is a valid pattern, and matches anywhere in
    /// the alias.
    ///
    /// Checked when an alias or a custom room ID is created, and at startup
    /// against the aliases already in the database, which are reported as
    /// warnings rather than removed.
    ///
    /// example: ["19dollarfortnitecards", "b[4a]droom", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_alias_names: RegexSet,

    /// Usernames that may not be registered, as regular expressions. A plain
    /// word is a valid pattern, and matches anywhere in the username.
    ///
    /// Checked on the availability request and on registration, and at
    /// startup against the users already in the database, which are reported
    /// as warnings rather than removed.
    ///
    /// example: ["administrator", "b[a4]dusernam[3e]", "badphrase"]
    ///
    /// default: []
    #[serde(default, with = "serde_regex")]
    pub forbidden_usernames: RegexSet,
}
