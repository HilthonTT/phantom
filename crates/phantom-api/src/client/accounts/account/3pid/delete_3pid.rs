use axum::extract::State;
use phantom_core::Result;
use phantom_service::auth::threepid::canonicalize_email;
use ruma::{
    api::client::account::{
        ThirdPartyIdRemovalStatus,
        delete_3pid::{self, v3::Response},
    },
    thirdparty::Medium,
};

use crate::router::Ruma;

/// # `POST /_matrix/client/v3/account/3pid/delete`
///
/// Remove a third party identifier from this account.
///
/// We never bound the address to an identity server, so the unbind result is
/// always `no-support`; any supplied `id_server` is ignored rather than
/// rejected.
#[tracing::instrument(skip_all, name = "delete_3pid")]
pub(crate) async fn delete_3pid_route(
    State(services): State<crate::router::State>,
    body: Ruma<delete_3pid::v3::Request>,
) -> Result<Response> {
    let sender_user = body.sender_user();

    // Phone numbers are stored, and listed to clients, as bare msisdn digits.
    let address = match body.medium {
        Medium::Msisdn => Some(body.address.trim_start_matches('+').to_owned()),
        _ => canonicalize_email(&body.address).ok(),
    };

    if let Some(address) = address {
        services.threepid.del_binding(sender_user, &address).await?;
    }

    Ok(Response::new(ThirdPartyIdRemovalStatus::NoSupport))
}
