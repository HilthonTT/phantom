//! Outbound SMTP email delivery.
//!
//! Opens its own TOML section rather than continuing `[global]`, so it is
//! declared after every module that does continue it.

use super::prelude::*;

/// Configures outbound email verification through SMTP.
///
/// The connection URI and sender identify the relay and source mailbox.
/// Registration flags control which flows require a verified email address.
#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", section = "global.smtp")]
pub struct SmtpConfig {
    /// Connection URL for the outbound SMTP relay used to send email
    /// verification messages. Setting this enables the email subsystem;
    /// without it no mail is sent.
    ///
    /// Use a `smtp://` URL for an unencrypted or STARTTLS connection and a
    /// `smtps://` URL for implicit TLS. Credentials and the host go inline:
    /// `smtps://user:pass@host:port`. The port defaults per scheme when
    /// omitted.
    ///
    /// The userinfo component is URL-encoded, so an `@` inside the username
    /// must be written as `%40` (for example a login of `bot@example.com`
    /// becomes `smtps://bot%40example.com:pass@host:465`). Other reserved
    /// characters in the username or password are percent-encoded the same
    /// way.
    ///
    /// example: "smtps://user:pass@mail.example.com:465"
    pub connection_uri: Option<String>,

    /// The mailbox that outbound verification messages are sent from. Accepts
    /// either a bare address or a display-name form.
    ///
    /// example: "Example <noreply@example.com>"
    pub sender: Option<String>,

    /// Require a verified email address to complete registration. When set,
    /// the registration flow does not finish until the user proves control of
    /// an email address.
    ///
    /// default: false
    #[serde(default)]
    pub require_email_for_registration: bool,

    /// Require a verified email address when registering with a registration
    /// token. When set, token-based registration also demands a verified
    /// email address.
    ///
    /// default: false
    #[serde(default)]
    pub require_email_for_token_registration: bool,
}
