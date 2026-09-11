use super::prelude::*;

#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", section = "global.smtp")]
pub struct SmtpConfig {
    pub connection_uri: Option<String>,

    pub sender: Option<String>,

    #[serde(default)]
    pub require_email_for_registration: bool,

    #[serde(default)]
    pub require_email_for_token_registration: bool,
}
