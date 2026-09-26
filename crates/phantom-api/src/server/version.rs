use phantom_core::{Result, diagnostics::info};
use ruma::api::federation::discovery::get_server_version;

use crate::router::Ruma;

pub(crate) async fn get_server_version_route(
    _body: Ruma<get_server_version::v1::Request>,
) -> Result<get_server_version::v1::Response> {
    let mut server = get_server_version::v1::Server::new();
    server.name = Some(info::name().into());
    server.version = Some(info::version().into());

    let mut response = get_server_version::v1::Response::new();
    response.server = Some(server);

    Ok(response)
}
