use super::*;

pub(super) async fn mainline_sort<E, F, Fut>(
    to_sort: &[E::Id],
    resolved_power_level: Option<E::Id>,
    fetch_event: &F,
    parallel_fetches: usize,
) -> Result<Vec<E::Id>>
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Clone + Send + Sync,
    E::Id: Borrow<EventId> + Clone + Send + Sync,
{
    debug!("mainline sort of events");

    if to_sort.is_empty() {
        return Ok(vec![]);
    }

    let mut mainline = vec![];
    let mut pl = resolved_power_level;
    while let Some(p) = pl {
        mainline.push(p.clone());

        let event = fetch_event(p.clone())
            .await
            .ok_or_else(|| Error::NotFound(format!("Failed to find {p}")))?;
        pl = None;
        for aid in event.auth_events() {
            let ev = fetch_event(aid.clone())
                .await
                .ok_or_else(|| Error::NotFound(format!("Failed to find {aid}")))?;
            if is_type_and_key(&ev, &TimelineEventType::RoomPowerLevels, "") {
                pl = Some(aid.to_owned());
                break;
            }
        }
    }

    let mainline_map = mainline
        .iter()
        .rev()
        .enumerate()
        .map(|(idx, eid)| ((*eid).clone(), idx))
        .collect::<HashMap<_, _>>();

    let order_map = stream::iter(to_sort.iter())
        .map(|ev_id| fetch_event(ev_id.clone()).map(move |event| event.map(|event| (event, ev_id))))
        .buffer_unordered(parallel_fetches)
        .filter_map(future::ready)
        .map(|(event, ev_id)| {
            get_mainline_depth(Some(event.clone()), &mainline_map, fetch_event)
                .map_ok(move |depth| (depth, event, ev_id))
                .map(Result::ok)
        })
        .buffer_unordered(parallel_fetches)
        .filter_map(future::ready)
        .fold(HashMap::new(), |mut order_map, (depth, event, ev_id)| {
            order_map.insert(ev_id, (depth, event.origin_server_ts(), ev_id));
            future::ready(order_map)
        })
        .boxed()
        .await;

    let mut sort_event_ids = order_map.keys().map(|&k| k.clone()).collect::<Vec<_>>();
    sort_event_ids.sort_by_key(|sort_id| &order_map[sort_id]);

    Ok(sort_event_ids)
}

async fn get_mainline_depth<E, F, Fut>(
    mut event: Option<E>,
    mainline_map: &HashMap<E::Id, usize>,
    fetch_event: &F,
) -> Result<usize>
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E: Event + Send + Sync,
    E::Id: Borrow<EventId> + Send + Sync,
{
    while let Some(sort_ev) = event {
        debug!(event_id = sort_ev.event_id().borrow().as_str(), "mainline");
        let id = sort_ev.event_id();
        if let Some(depth) = mainline_map.get(id.borrow()) {
            return Ok(*depth);
        }

        event = None;
        for aid in sort_ev.auth_events() {
            let aev = fetch_event(aid.clone())
                .await
                .ok_or_else(|| Error::NotFound(format!("Failed to find {aid}")))?;
            if is_type_and_key(&aev, &TimelineEventType::RoomPowerLevels, "") {
                event = Some(aev);
                break;
            }
        }
    }
    Ok(0)
}
