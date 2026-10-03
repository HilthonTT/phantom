use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::api::client::keys::upload_signing_keys;

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

    let unchanged = match (&existing_master, &body.master_key) {
        (Some(existing), Some(new)) => existing.json().get() == new.json().get(),
        (Some(_), None) => true,
        (None, _) => false,
    };

    // A first upload, or a re-upload of the same master key, needs no UIAA.
    if existing_master.is_some()
        && !unchanged
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
