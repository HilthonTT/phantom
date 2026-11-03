use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::{api::client::keys::upload_signing_keys, encryption::CrossSigningKey, serde::Raw};
use serde_json::Value as JsonValue;

use crate::router::{Ruma, authenticate_uiaa};

/// # `POST /_matrix/client/r0/keys/device_signing/upload`
///
/// Uploads end-to-end key information for the sender user.
///
/// - Requires UIAA to replace an existing master key, unless the replacement
///   was approved out of band (MSC4312)
pub(crate) async fn upload_signing_keys_route(
    State(services): State<crate::router::State>,
    body: Ruma<upload_signing_keys::v3::Request>,
) -> Result<upload_signing_keys::v3::Response> {
    let sender_user = body.sender_user();

    let existing_master = services
        .users
        .get_master_key(None, sender_user, &|_| true)
        .await
        .ok();

    let existing_self_signing = services
        .users
        .get_self_signing_key(None, sender_user, &|_| true)
        .await
        .ok();

    let existing_user_signing = services.users.get_user_signing_key(sender_user).await.ok();

    // Omitted keys are left untouched; any supplied key that differs from the
    // stored one is a replacement.
    let replaces_keys = is_replaced(existing_master.as_ref(), body.master_key.as_ref())
        || is_replaced(
            existing_self_signing.as_ref(),
            body.self_signing_key.as_ref(),
        )
        || is_replaced(
            existing_user_signing.as_ref(),
            body.user_signing_key.as_ref(),
        );

    // A first upload, or a re-upload of the same keys, needs no UIAA.
    if existing_master.is_some()
        && replaces_keys
        && !services.users.can_replace_cross_signing_keys(sender_user)
    {
        authenticate_uiaa(&services, &body).await?;
    }

    if body.master_key.is_none() && existing_master.is_none() {
        return Err!(Request(MissingParam(
            "Tried to upload signing keys without a master key."
        )));
    }

    services
        .users
        .add_cross_signing_keys(
            sender_user,
            &body.master_key,
            &body.self_signing_key,
            &body.user_signing_key,
            true,
        )
        .await?;

    Ok(upload_signing_keys::v3::Response::new())
}

fn is_replaced(
    existing: Option<&Raw<CrossSigningKey>>,
    new: Option<&Raw<CrossSigningKey>>,
) -> bool {
    let Some(new) = new else {
        return false;
    };

    let parse =
        |key: &Raw<CrossSigningKey>| serde_json::from_str::<JsonValue>(key.json().get()).ok();

    existing.is_none_or(|existing| {
        let existing = parse(existing);
        existing.is_none() || existing != parse(new)
    })
}
