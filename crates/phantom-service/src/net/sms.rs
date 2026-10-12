//! Text messages, sent through the HTTP gateway `[global.sms]` names; they
//! carry the codes that verify phone numbers.

use std::sync::Arc;

use phantom_core::{Err, Result, err, http, implement, runtime::server::Server};
use serde_json::json;

use crate::{Dep, net::client};

pub struct Service {
    services: Services,
}

struct Services {
    server: Arc<Server>,
    client: Dep<client::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                client: args.depend::<client::Service>(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Whether a gateway is configured, so phone numbers can be verified.
#[implement(Service)]
#[must_use]
pub fn is_enabled(&self) -> bool {
    self.services.server.config.sms.gateway_url.is_some()
}

/// Texts a verification code to `msisdn`, the number's digits without a `+`.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, code))]
pub async fn send_code(&self, msisdn: &str, code: &str) -> Result {
    let config = &self.services.server.config;
    let message = config
        .sms
        .message
        .replace("{code}", code)
        .replace("{server_name}", config.server_name.as_str());

    self.send(msisdn, &message).await
}

/// Sends `message` to `msisdn` through the gateway.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, message))]
pub async fn send(&self, msisdn: &str, message: &str) -> Result {
    let sms = &self.services.server.config.sms;
    let Some(url) = &sms.gateway_url else {
        return Err!(Config("sms.gateway_url", "No SMS gateway is configured"));
    };

    let mut request = self
        .services
        .client
        .default
        .post(url.clone())
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(json!({ "to": format!("+{msisdn}"), "message": message }).to_string());

    if let Some(token) = &sms.gateway_token {
        request = request.bearer_auth(token);
    }

    request
        .send()
        .await
        .map_err(|e| err!(Request(Unknown("Failed to reach the SMS gateway: {e}"))))?
        .error_for_status()
        .map_err(|e| err!(Request(Unknown("The SMS gateway refused the message: {e}"))))?;

    Ok(())
}
