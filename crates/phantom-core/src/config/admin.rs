//! The admin room and the commands it accepts.
//!
//! These are `[global]` keys like any other. The struct exists to keep one
//! subject in one file; `#[serde(flatten)]` folds it back into
//! [`Config`](super::Config), so the TOML is unchanged.

use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Admin {
    /// Password set on the server's own user account so an operator locked out
    /// of every admin account can log in as it and recover one.
    ///
    /// While this is set the server user is a usable account with the default
    /// push ruleset. Unset it once recovery is done: clearing it deactivates
    /// the account again and logs out every session that was opened with it.
    ///
    /// display: sensitive
    pub emergency_password: Option<String>,

    /// Admin commands to run once the server has started, in order, as if
    /// they had been typed into the admin room.
    ///
    /// Each entry is one command without its `!admin` prefix. Their output
    /// goes to the log, since there is nobody in a room to answer, and a
    /// command that fails stops startup unless
    /// `admin_execute_errors_ignore` is set.
    ///
    /// This build registers no command set, so anything listed here fails.
    /// The option is here because the schedule belongs to the admin service
    /// and the commands do not.
    ///
    /// example: ["users create-user @admin:example.com", "server memory-usage"]
    ///
    /// default: []
    #[serde(default)]
    pub admin_execute: Vec<String>,

    /// Admin commands to run every time the server is sent SIGUSR2, in the
    /// same form as `admin_execute`.
    ///
    /// Unlike the startup list this one is re-read each time, so a reloaded
    /// config changes what the next signal runs.
    ///
    /// default: []
    #[serde(default)]
    pub admin_signal_execute: Vec<String>,

    /// Carry on when one of the commands above fails, instead of treating the
    /// failure as fatal to startup.
    ///
    /// default: false
    #[serde(default)]
    pub admin_execute_errors_ignore: bool,

    /// Let an admin run a command outside the admin room by escaping it with a
    /// backslash, as `\!admin ...`.
    ///
    /// The command and its output are both visible to that room, which is the
    /// point: it is how an admin answers a question where it was asked. Only
    /// local admins can do it, escaped or not.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub admin_escape_commands: bool,

    /// Reload the configuration when the server is sent SIGUSR1.
    ///
    /// Only `server_name` is fixed for the life of the process; every other
    /// option is re-read. Has no effect where the platform has no SIGUSR1.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub config_reload_signal: bool,
}
