use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Instant,
};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use phantom_core::{
    Result, debug, diagnostics::log::debug::INFO_SPAN_LEVEL,
    matrix::state_res::lexicographical_topological_sort,
};
use phantom_service::Services;
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId,
    ServerName, TransactionId, int, uint,
};

pub(super) type ResolvedMap = BTreeMap<OwnedEventId, Result>;
type IncomingPdu = (OwnedRoomId, OwnedEventId, CanonicalJsonObject);

pub(super) type IndexedPdu = (usize, IncomingPdu);

pub(super) async fn handle(
    services: &Services,
    origin: &ServerName,
    txn_id: &TransactionId,
    pdus: Vec<IndexedPdu>,
) -> Result<ResolvedMap> {
    let mut rooms: BTreeMap<OwnedRoomId, Vec<IndexedPdu>> = BTreeMap::new();
    for pdu in pdus {
        rooms.entry(pdu.1.0.clone()).or_default().push(pdu);
    }

    let mut handled = FuturesUnordered::new();
    for (room_id, pdus) in rooms {
        handled.push(handle_room(services, origin, txn_id, room_id, pdus));
    }

    let mut resolved = ResolvedMap::new();
    while let Some(results) = handled.next().await {
        resolved.extend(results?);
    }

    Ok(resolved)
}

#[tracing::instrument(name = "room", level = INFO_SPAN_LEVEL, skip_all, fields(%room_id))]
async fn handle_room(
    services: &Services,
    origin: &ServerName,
    txn_id: &TransactionId,
    room_id: OwnedRoomId,
    pdus: Vec<IndexedPdu>,
) -> Result<ResolvedMap> {
    let pdus = sort_pdus(pdus).await;
    let federation_lock = services.rooms.event_handler.lock_federation(&room_id).await;

    let mut results = ResolvedMap::new();

    for (txn_index, (_, event_id, value)) in pdus {
        services.server.check_running()?;

        let started = Instant::now();
        let result = services
            .rooms
            .event_handler
            .handle_incoming_pdu(&federation_lock, origin, &room_id, &event_id, value, true)
            .map(|result| result.map(drop))
            .await;

        debug!(%txn_id, %event_id, txn_index, elapsed = ?started.elapsed(), "Finished PDU");

        results.insert(event_id, result);
    }

    Ok(results)
}

pub(super) async fn sort_pdus(mut pdus: Vec<IndexedPdu>) -> Vec<IndexedPdu> {
    if is_sorted(&pdus) {
        return pdus;
    }

    let batch: HashMap<&str, &OwnedEventId> = pdus
        .iter()
        .map(|(_, (_, event_id, _))| (event_id.as_str(), event_id))
        .collect();

    let graph: HashMap<OwnedEventId, HashSet<OwnedEventId>> = pdus
        .iter()
        .map(|(_, (_, event_id, value))| {
            let prev_events = prev_event_ids(value)
                .filter_map(|prev| batch.get(prev).map(|&id| id.clone()))
                .collect();

            (event_id.clone(), prev_events)
        })
        .collect();

    let tie_breaker = async |_: OwnedEventId| Ok((int!(0), MilliSecondsSinceUnixEpoch(uint!(0))));

    let Ok(order) = lexicographical_topological_sort(&graph, &tie_breaker).await else {
        return pdus;
    };

    let position: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(index, event_id)| (event_id.as_str(), index))
        .collect();

    pdus.sort_by_key(|(_, (_, event_id, _))| position.get(event_id.as_str()).copied());
    pdus
}

fn is_sorted(pdus: &[IndexedPdu]) -> bool {
    let position: HashMap<&str, usize> = pdus
        .iter()
        .enumerate()
        .map(|(index, (_, (_, event_id, _)))| (event_id.as_str(), index))
        .collect();

    pdus.iter().enumerate().all(|(index, (_, (_, _, value)))| {
        prev_event_ids(value).all(|prev| position.get(prev).is_none_or(|&prev| prev < index))
    })
}

fn prev_event_ids(value: &CanonicalJsonObject) -> impl Iterator<Item = &str> + '_ {
    value
        .get("prev_events")
        .and_then(CanonicalJsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(CanonicalJsonValue::as_str)
}

#[cfg(test)]
mod tests {
    use ruma::{CanonicalJsonObject, OwnedEventId, event_id, room_id};
    use serde_json::json;

    use super::{IndexedPdu, is_sorted, prev_event_ids, sort_pdus};

    fn pdu(index: usize, id: &OwnedEventId, prev: &[&OwnedEventId]) -> IndexedPdu {
        let prev_events: Vec<&str> = prev.iter().map(|id| id.as_str()).collect();
        let value: CanonicalJsonObject =
            serde_json::from_value(json!({ "prev_events": prev_events }))
                .expect("valid canonical json");

        (
            index,
            (room_id!("!r:example.com").to_owned(), id.clone(), value),
        )
    }

    fn ids() -> (OwnedEventId, OwnedEventId, OwnedEventId) {
        (
            event_id!("$a:example.com").to_owned(),
            event_id!("$b:example.com").to_owned(),
            event_id!("$c:example.com").to_owned(),
        )
    }

    fn order(pdus: &[IndexedPdu]) -> Vec<&str> {
        pdus.iter().map(|(_, (_, id, _))| id.as_str()).collect()
    }

    #[test]
    fn sorted_when_parents_lead() {
        let (a, b, c) = ids();
        assert!(is_sorted(&[
            pdu(0, &a, &[]),
            pdu(1, &b, &[&a]),
            pdu(2, &c, &[&b])
        ]));
    }

    #[test]
    fn unsorted_when_child_leads() {
        let (a, b, _) = ids();
        assert!(!is_sorted(&[pdu(0, &b, &[&a]), pdu(1, &a, &[])]));
    }

    #[test]
    fn sorted_ignores_out_of_batch_references() {
        let (a, b, c) = ids();
        assert!(is_sorted(&[pdu(0, &b, &[&c]), pdu(1, &a, &[&c])]));
    }

    #[tokio::test]
    async fn sort_orders_parents_before_children() {
        let (a, b, c) = ids();
        let sorted = sort_pdus(vec![pdu(0, &c, &[&b]), pdu(1, &b, &[&a]), pdu(2, &a, &[])]).await;

        assert_eq!(
            order(&sorted),
            ["$a:example.com", "$b:example.com", "$c:example.com"]
        );
    }

    #[tokio::test]
    async fn sort_is_noop_when_already_ordered() {
        let (a, b, c) = ids();
        let pdus = vec![pdu(0, &a, &[]), pdu(1, &b, &[&a]), pdu(2, &c, &[&b])];
        let sorted = sort_pdus(pdus.clone()).await;

        assert_eq!(order(&sorted), order(&pdus));
    }

    #[tokio::test]
    async fn sort_preserves_a_cycle() {
        let (a, b, _) = ids();
        let sorted = sort_pdus(vec![pdu(0, &a, &[&b]), pdu(1, &b, &[&a])]).await;

        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn prev_event_ids_reads_the_array() {
        let (a, b, _) = ids();
        let (_, (_, _, value)) = pdu(0, &a, &[&b]);

        assert_eq!(
            prev_event_ids(&value).collect::<Vec<_>>(),
            ["$b:example.com"]
        );
    }

    #[test]
    fn prev_event_ids_empty_when_absent() {
        assert_eq!(prev_event_ids(&CanonicalJsonObject::new()).count(), 0);
    }
}
