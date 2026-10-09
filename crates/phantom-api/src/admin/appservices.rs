use axum::{Json, extract::State, response::IntoResponse};
use phantom_core::Result;
use ruma::api::appservice::{Namespace, Registration};
use serde::Serialize;

use crate::router::{AdminAuth, State as RouterState};

/// A registration as the console shows it: everything but the two tokens,
/// which would let whoever reads them act as the appservice or the server.
#[derive(Serialize)]
pub(super) struct Appservice {
    id: String,
    url: Option<String>,
    sender_localpart: String,

    users: Vec<String>,
    aliases: Vec<String>,
    rooms: Vec<String>,

    rate_limited: bool,
    protocols: Vec<String>,
}

/// # `GET /_phantom/admin/v1/appservices`
///
/// Every registered appservice.
pub(super) async fn appservices(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let appservices: Vec<Appservice> = services
        .appservice
        .all()
        .await
        .into_iter()
        .map(|(_, registration)| appservice(registration))
        .collect();

    Ok(Json(appservices))
}

fn appservice(registration: Registration) -> Appservice {
    let regexes = |namespaces: &[Namespace]| -> Vec<String> {
        namespaces
            .iter()
            .map(|namespace| namespace.regex.clone())
            .collect()
    };

    Appservice {
        users: regexes(&registration.namespaces.users),
        aliases: regexes(&registration.namespaces.aliases),
        rooms: regexes(&registration.namespaces.rooms),
        id: registration.id,
        url: registration.url,
        sender_localpart: registration.sender_localpart,
        // The spec has an appservice rate limited unless it says otherwise.
        rate_limited: registration.rate_limited.unwrap_or(true),
        protocols: registration.protocols.unwrap_or_default(),
    }
}
