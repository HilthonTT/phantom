//! Checking whether a newer phantom has been released.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Updates {
    /// Periodically fetch phantom's announcement feed, which carries security
    /// and release notices. Despite the name this checks for announcements,
    /// not for a newer version to install.
    #[serde(default)]
    pub allow_check_for_updates: bool,

    /// Where the announcement feed is fetched from.
    ///
    /// The feed is a JSON document of `{"announcements": [{"id": 1, "message":
    /// "..."}]}`, read in ascending `id` order; an operator running their own
    /// fork points this at their own file.
    ///
    /// default: "https://raw.githubusercontent.com/HilthonTT/phantom/main/announcements.json"
    #[serde(default = "default_check_for_updates_url")]
    pub check_for_updates_url: String,

    /// How long to wait between fetches of the announcement feed, in seconds.
    ///
    /// default: 7200
    #[serde(default = "default_check_for_updates_interval_s")]
    pub check_for_updates_interval_s: u64,
}
