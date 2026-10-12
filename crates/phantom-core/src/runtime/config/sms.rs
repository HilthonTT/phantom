use super::prelude::*;

/// Sends the codes that verify phone numbers (the `msisdn` medium), through an
/// HTTP gateway of your choosing.
///
/// For each code phantom POSTs the gateway the JSON
/// `{"to": "+15551234567", "message": "..."}` and counts any 2xx answer as
/// sent. Point it at your SMS provider's API, or at a small relay that turns
/// that request into the provider's own. Phone verification stays off while
/// `gateway_url` is unset.
#[derive(Clone, Debug, Default, Deserialize)]
#[config_example_generator(filename = "phantom-example.toml", section = "global.sms")]
pub struct SmsConfig {
    /// The gateway's URL.
    ///
    /// example: "https://sms-relay.example.com/send"
    pub gateway_url: Option<Url>,

    /// Sent to the gateway as `Authorization: Bearer <gateway_token>`.
    #[doc = "display: sensitive"]
    pub gateway_token: Option<String>,

    /// The text message; `{code}` becomes the verification code and
    /// `{server_name}` this server's name.
    ///
    /// default: "Your {server_name} verification code is {code}"
    #[serde(default = "default_sms_message")]
    pub message: String,
}

fn default_sms_message() -> String {
    "Your {server_name} verification code is {code}".to_owned()
}
