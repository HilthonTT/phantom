use super::prelude::*;

#[derive(Clone, Debug, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", continues = "global")]
pub struct Admin {
    #[doc = "display: sensitive"]
    pub emergency_password: Option<String>,

    #[serde(default)]
    pub admin_execute: Vec<String>,

    #[serde(default)]
    pub admin_signal_execute: Vec<String>,

    #[serde(default)]
    pub admin_execute_errors_ignore: bool,

    #[serde(default = "true_fn")]
    pub admin_escape_commands: bool,

    #[serde(default = "true_fn")]
    pub config_reload_signal: bool,

    /// Create the admin room (`#admins:<server_name>`) and the server user on
    /// a fresh database. The first user to register then joins it as admin.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub create_admin_room: bool,

    /// Room tag set on the admin room for each user granted admin, so clients
    /// file it apart. Empty sets no tag.
    ///
    /// default: "m.server_notice"
    #[serde(default = "default_admin_room_tag")]
    pub admin_room_tag: String,

    /// Post a welcome notice into the admin room when a user is granted admin.
    ///
    /// default: true
    #[serde(default = "true_fn")]
    pub admin_room_notices: bool,
}
