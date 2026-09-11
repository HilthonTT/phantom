//! Presence, read receipts, and typing notifications.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Presence {
    /// Seconds a user may be idle before their presence is moved from
    /// "online" to "unavailable".
    ///
    /// The clock starts at the last presence update the user's client sent,
    /// so a client that pings while a person is away keeps them online.
    ///
    /// default: 300
    #[serde(default = "default_presence_idle_timeout_s")]
    pub presence_idle_timeout_s: u64,

    /// Seconds a user may stay "unavailable" before their presence is moved
    /// to "offline".
    ///
    /// Measured from the same last update as `presence_idle_timeout_s`
    /// rather than from the move to "unavailable", so it wants to be the
    /// larger of the two.
    ///
    /// default: 1800
    #[serde(default = "default_presence_offline_timeout_s")]
    pub presence_offline_timeout_s: u64,

    /// Time out remote users' presence as well as local users'.
    ///
    /// A remote server sends presence for its own users and stops sending
    /// when they go quiet, which leaves them showing as online here forever.
    /// Timing them out locally is what clears that, at the cost of a timer
    /// per remote user this server has heard about.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub presence_timeout_remote_users: bool,

    /// Send local users' presence to the other servers in their rooms.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_outgoing_presence: bool,

    /// Send local users' read receipts to the other servers in their rooms.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_outgoing_read_receipts: bool,

    /// Send local users' typing notifications to the other servers in their
    /// rooms.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub allow_outgoing_typing: bool,
}
