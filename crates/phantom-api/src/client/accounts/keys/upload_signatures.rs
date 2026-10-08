use std::collections::BTreeMap;

use axum::extract::State;
use phantom_core::{Result, debug_warn};
use ruma::{
    OwnedUserId,
    api::client::keys::upload_signatures::{self, v3::Failure},
};
use serde_json::{Value as JsonValue, json};

use crate::router::Ruma;

/// # `POST /_matrix/client/r0/keys/signatures/upload`
///
/// Uploads end-to-end key signatures from the sender user.
///
/// A signature that cannot be stored is reported in `failures` rather than
/// failing the whole request.
pub(crate) async fn upload_signatures_route(
    State(services): State<crate::router::State>,
    body: Ruma<upload_signatures::v3::Request>,
) -> Result<upload_signatures::v3::Response> {
    let sender_user = body.sender_user();
    let mut failures: BTreeMap<OwnedUserId, BTreeMap<String, Failure>> = BTreeMap::new();

    for (user_id, keys) in &body.signed_keys {
        for (key_id, key) in keys {
            let signatures = serde_json::from_str::<JsonValue>(key.get())
                .ok()
                .and_then(|key| key.get("signatures")?.get(sender_user.as_str()).cloned())
                .and_then(|signatures| signatures.as_object().cloned())
                .unwrap_or_default();

            for (signature_id, signature) in signatures {
                let Some(signature) = signature.as_str() else {
                    continue;
                };

                let signature = (signature_id, signature.to_owned());
                if let Err(e) = services
                    .users
                    .sign_key(user_id, key_id, signature, sender_user)
                    .await
                {
                    debug_warn!(%user_id, %key_id, "Failed to store signature: {e}");

                    // ruma's Failure has no constructor; it only deserializes.
                    if let Ok(failure) = serde_json::from_value::<Failure>(json!({
                        "errcode": "M_INVALID_SIGNATURE",
                        "error": e.message(),
                    })) {
                        failures
                            .entry(user_id.clone())
                            .or_default()
                            .insert(key_id.to_owned(), failure);
                    }
                }
            }
        }
    }

    let mut response = upload_signatures::v3::Response::new();
    response.failures = failures;

    Ok(response)
}
