//! Step 3 of resolving a server name: asking the name itself where its
//! homeserver lives.

use phantom_core::{Result, debug, debug_error, debug_info, debug_warn, implement, trace};
use ruma::ServerName;

/// The delegated server name `dest` publishes, if it publishes a usable one.
///
/// Every failure here is `Ok(None)` rather than an error: not publishing a
/// `.well-known` is the normal case, and a malformed or oversized one is
/// answered the same way the spec answers a missing one — by moving on to the
/// SRV lookup.
#[implement(super::Service)]
#[tracing::instrument(name = "well-known", level = "debug", skip(self, dest))]
pub(super) async fn request_well_known(&self, dest: &str) -> Result<Option<String>> {
    trace!("Requesting well-known for {dest}");

    let response = self
        .services
        .client
        .well_known
        .get(format!("https://{dest}/.well-known/matrix/server"))
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(e) => {
            debug!("Well-known request to {dest:?} failed: {e}");
            return Ok(None);
        }
    };

    if !response.status().is_success() {
        debug!("Well-known for {dest:?} answered {}", response.status());
        return Ok(None);
    }

    let text = match response.text().await {
        Ok(text) => text,
        Err(e) => {
            debug!("Well-known body from {dest:?} could not be read: {e}");
            return Ok(None);
        }
    };
    trace!("response text: {text:?}");

    if text.len() >= MAX_RESPONSE_LEN {
        debug_warn!(
            "Well-known for {dest:?} is {} bytes; ignoring it",
            text.len()
        );
        return Ok(None);
    }

    let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) else {
        debug_error!("Well-known for {dest:?} is not JSON");
        return Ok(None);
    };

    let m_server = body.get("m.server").and_then(serde_json::Value::as_str);

    let Some(m_server) = m_server.filter(|name| ServerName::parse(*name).is_ok()) else {
        debug_error!("Well-known for {dest:?} has no valid \"m.server\"");
        return Ok(None);
    };

    debug_info!("{dest:?} found at {m_server:?}");
    Ok(Some(m_server.to_owned()))
}

/// A `.well-known` file is a single short JSON object. Anything approaching
/// this is not one, and is not worth parsing to find that out.
const MAX_RESPONSE_LEN: usize = 12288;
