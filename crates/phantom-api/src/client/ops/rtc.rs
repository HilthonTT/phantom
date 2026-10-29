use axum::extract::State;
use phantom_core::{Config, Result, err};
use ruma::api::client::rtc::{RtcTransport, transports};

use crate::router::Ruma;

/// # `GET /_matrix/client/unstable/org.matrix.msc4143/rtc/transports`
///
/// The MatrixRTC transports group calls should use, from `rtc_transports`.
pub(crate) async fn get_rtc_transports_route(
    State(services): State<crate::router::State>,
    _body: Ruma<transports::v1::Request>,
) -> Result<transports::v1::Response> {
    Ok(transports::v1::Response::new(rtc_transports(
        &services.config,
    )?))
}

/// The configured transports as ruma's type: `type` names the transport and
/// the rest of the table is its data. Config validation has already refused
/// a transport without a string `type`.
pub(crate) fn rtc_transports(config: &Config) -> Result<Vec<RtcTransport>> {
    config
        .client
        .rtc_transports
        .iter()
        .map(|transport| {
            let mut data = transport.clone();
            let kind = data
                .remove("type")
                .and_then(|kind| kind.as_str().map(ToOwned::to_owned))
                .unwrap_or_default();

            RtcTransport::new(&kind, data)
                .map_err(|e| err!(Config("rtc_transports", "invalid {kind} transport: {e}")))
        })
        .collect()
}
