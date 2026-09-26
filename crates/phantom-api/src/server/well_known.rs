use axum::extract::State;
use phantom_core::{Result, err};
use ruma::api::federation::discovery::discover_homeserver;

use crate::router::Ruma;

pub(crate) async fn well_known_server(
    State(services): State<crate::router::State>,
    _body: Ruma<discover_homeserver::Request>,
) -> Result<discover_homeserver::Response> {
    let server = services
        .server
        .config
        .auth
        .well_known_server
        .clone()
        .ok_or_else(|| err!(Request(NotFound("Not found."))))?;

    Ok(discover_homeserver::Response::new(server))
}
