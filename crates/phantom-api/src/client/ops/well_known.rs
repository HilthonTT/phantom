use axum::extract::State;
use phantom_core::{Err, Result};
use ruma::{
    OwnedUserId,
    api::client::discovery::{
        discover_homeserver::{self, HomeserverInfo},
        discover_support::{self, Contact, ContactRole},
    },
};

use crate::router::Ruma;

/// # `GET /.well-known/matrix/client`
///
/// Returns the .well-known URL if it is configured, otherwise returns 404.
pub(crate) async fn well_known_client(
    State(services): State<crate::router::State>,
    _body: Ruma<discover_homeserver::Request>,
) -> Result<discover_homeserver::Response> {
    let Some(url) = services.config.auth.well_known_client.as_ref() else {
        return Err!(Request(NotFound("Not found.")));
    };

    let homeserver = HomeserverInfo::new(url.to_string());

    Ok(discover_homeserver::Response::new(homeserver))
}

/// # `GET /.well-known/matrix/support`
///
/// Server support contact and support page of a homeserver's domain.
pub(crate) async fn well_known_support(
    State(services): State<crate::router::State>,
    _body: Ruma<discover_support::Request>,
) -> Result<discover_support::Response> {
    let config = &services.config.client;

    let support_page = config
        .well_known_support_page
        .as_ref()
        .map(ToString::to_string);

    let role = config
        .well_known_support_role
        .as_deref()
        .map_or(ContactRole::Admin, ContactRole::from);

    let email_address = config.well_known_support_email.clone();

    let matrix_id = config
        .well_known_support_mxid
        .as_deref()
        .map(OwnedUserId::try_from)
        .transpose()
        .map_err(|e| phantom_core::err!(Config("well_known_support_mxid", "{e}")))?;

    let contacts = match (email_address, matrix_id) {
        (None, None) => Vec::new(),
        (email_address, matrix_id) => {
            let mut contact = match (&email_address, &matrix_id) {
                (Some(email), _) => Contact::with_email_address(role, email.clone()),
                (None, Some(mxid)) => Contact::with_matrix_id(role, mxid.clone()),
                (None, None) => unreachable!(),
            };
            contact.email_address = email_address;
            contact.matrix_id = matrix_id;
            vec![contact]
        }
    };

    if support_page.is_none() && contacts.is_empty() {
        return Err!(Request(NotFound("Not found.")));
    }

    let mut response = discover_support::Response::with_contacts(contacts);
    response.support_page = support_page;

    Ok(response)
}
