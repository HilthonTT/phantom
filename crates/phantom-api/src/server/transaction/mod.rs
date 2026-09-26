mod edus;
mod pdus;

use std::time::Instant;

use axum::extract::State;
use futures::FutureExt;
use phantom_core::{
    Err, Error, Result, debug, debug_warn,
    diagnostics::{error::sanitized_message, log::debug::INFO_SPAN_LEVEL},
    trace, warn,
};
use ruma::api::{
    error::ErrorKind,
    federation::transactions::{edu::Edu, send_transaction_message},
};

use crate::router::{ClientIp, Ruma};

const PDU_LIMIT: usize = 50;
const EDU_LIMIT: usize = 100;

#[tracing::instrument(
    name = "txn",
    level = INFO_SPAN_LEVEL,
    skip_all,
    fields(txn = %body.transaction_id, origin = %body.origin(), %client),
)]
pub(crate) async fn send_transaction_message_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<send_transaction_message::v1::Request>,
) -> Result<send_transaction_message::v1::Response> {
    let origin = body.origin();

    if origin != body.body.origin {
        return Err!(Request(Forbidden(
            "Not allowed to send transactions on behalf of other servers"
        )));
    }

    if body.pdus.len() > PDU_LIMIT {
        return Err!(Request(Forbidden(
            "Not allowed to send more than {PDU_LIMIT} PDUs in one transaction"
        )));
    }

    if body.edus.len() > EDU_LIMIT {
        return Err!(Request(Forbidden(
            "Not allowed to send more than {EDU_LIMIT} EDUs in one transaction"
        )));
    }

    services.sending.notify_peer_alive(origin).await;

    let started = Instant::now();
    trace!(
        pdus = body.pdus.len(),
        edus = body.edus.len(),
        "Starting txn"
    );

    let mut pdus = Vec::with_capacity(body.pdus.len());
    for (index, pdu) in body.pdus.iter().enumerate() {
        match services.rooms.event_handler.parse_incoming_pdu(pdu).await {
            Ok((event_id, value, room_id)) => pdus.push((index, (room_id, event_id, value))),
            Err(e) => debug_warn!("Could not parse PDU[{index}]: {e}"),
        }
    }

    let edus: Vec<Edu> = body
        .edus
        .iter()
        .enumerate()
        .filter_map(|(index, edu)| {
            serde_json::from_str(edu.json().get())
                .inspect_err(|e| debug_warn!("Could not parse EDU[{index}]: {e}"))
                .ok()
        })
        .collect();

    let results = pdus::handle(&services, origin, &body.transaction_id, pdus)
        .boxed()
        .await?;

    edus::handle(&services, origin, edus).await;

    debug!(
        pdus = body.pdus.len(),
        edus = body.edus.len(),
        elapsed = ?started.elapsed(),
        "Finished txn",
    );

    for (event_id, result) in &results {
        if let Err(e @ Error::BadRequest(ErrorKind::NotFound, _)) = result {
            warn!("Incoming PDU failed {event_id}: {e:?}");
        }
    }

    let pdus = results
        .into_iter()
        .map(|(event_id, result)| (event_id, result.map_err(sanitized_message)))
        .collect();

    Ok(send_transaction_message::v1::Response::new(pdus))
}
