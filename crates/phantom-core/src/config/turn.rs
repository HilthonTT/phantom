//! The TURN server clients are handed for voice and video.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Turn {
    /// Static TURN username handed to clients, for a TURN server that
    /// authenticates with fixed credentials rather than `turn_secret`.
    ///
    /// default: ""
    #[serde(default)]
    pub turn_username: String,

    /// Static TURN password handed to clients. See `turn_username`.
    ///
    /// display: sensitive
    /// default: ""
    #[serde(default)]
    pub turn_password: String,

    /// TURN servers to hand to clients, as URIs. Use the `turns:` scheme
    /// rather than `turn:` for TURN over TLS.
    ///
    /// example: ["turn:example.turn.uri?transport=udp",
    /// "turn:example.turn.uri?transport=tcp"]
    ///
    /// default: []
    #[serde(default)]
    pub turn_uris: Vec<String>,

    /// Shared secret the TURN server is configured with, from which phantom
    /// derives the time-limited credentials it hands each client.
    ///
    /// Preferred over the static `turn_username`/`turn_password` pair, since
    /// a credential phantom derives expires on its own.
    ///
    /// display: sensitive
    /// default: ""
    #[serde(default)]
    pub turn_secret: String,

    /// Path to a file holding the TURN shared secret instead of writing it
    /// into the config. The contents are read once at startup, with
    /// surrounding whitespace trimmed, and take priority over `turn_secret`;
    /// a file that cannot be read falls back to it.
    ///
    /// example: "/etc/phantom/.turn_secret"
    pub turn_secret_file: Option<PathBuf>,

    /// How long, in seconds, a TURN credential phantom derives stays valid.
    ///
    /// default: 86400
    #[serde(default = "default_turn_ttl")]
    pub turn_ttl: u64,
}
