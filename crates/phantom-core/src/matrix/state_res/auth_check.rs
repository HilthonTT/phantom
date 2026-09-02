use super::*;

/// Check the that each event is authenticated based on the events before it.
///
/// ## Returns
///
/// The `unconflicted_state` combined with the newly auth'ed events. So any
/// event that fails the `event_auth::auth_check` will be excluded from the
/// returned state map.
///
/// For each `events_to_check` event we gather the events needed to auth it from
/// the the `fetch_event` closure and verify each event using the
/// `event_auth::auth_check` function.
pub(super) async fn iterative_auth_check<'a, E, F, Fut, I>(
    room_version: &RoomVersion,
    events_to_check: I,
    unconflicted_state: StateMap<E::Id>,
    fetch_event: &F,
    parallel_fetches: usize,
) -> Result<StateMap<E::Id>>
where
    F: Fn(E::Id) -> Fut + Sync,
    Fut: Future<Output = Option<E>> + Send,
    E::Id: Borrow<EventId> + Clone + Eq + Ord + Send + Sync + 'a,
    I: Iterator<Item = &'a E::Id> + Debug + Send + 'a,
    E: Event + Clone + Send + Sync,
{
    debug!("starting iterative auth check");
    trace!(
        list = ?events_to_check,
        "events to check"
    );

    let events_to_check: Vec<_> = stream::iter(events_to_check)
        .map(Result::Ok)
        .map_ok(|event_id| {
            fetch_event(event_id.clone()).map(move |result| {
                result.ok_or_else(|| Error::NotFound(format!("Failed to find {event_id}")))
            })
        })
        // Diverges from upstream's `try_buffer_unordered`: the events must be
        // auth-checked in the order the sort produced, and an unordered fetch
        // yielded them in I/O completion order, which made the resolved state
        // depend on database timing whenever `parallel_fetches > 1`.
        .try_buffered(parallel_fetches)
        .try_collect()
        .boxed()
        .await?;

    let auth_event_ids: HashSet<E::Id> = events_to_check
        .iter()
        .flat_map(|event: &E| event.auth_events().map(Clone::clone))
        .collect();

    let auth_events: HashMap<E::Id, E> = stream::iter(auth_event_ids)
        .map(fetch_event)
        .buffer_unordered(parallel_fetches)
        .filter_map(future::ready)
        .map(|auth_event| (auth_event.event_id().clone(), auth_event))
        .collect()
        .boxed()
        .await;

    let auth_events = &auth_events;
    let mut resolved_state = unconflicted_state;
    for event in &events_to_check {
        let event_id = event.event_id();
        let state_key = event
            .state_key()
            .ok_or_else(|| Error::InvalidPdu("State event had no state key".to_owned()))?;

        let auth_types = auth_types_for_event(
            event.event_type(),
            event.sender(),
            Some(state_key),
            event.content(),
        )?;

        let mut auth_state = StateMap::new();
        for aid in event.auth_events() {
            if let Some(ev) = auth_events.get(aid.borrow()) {
                auth_state.insert(
                    ev.event_type()
                        .with_state_key(ev.state_key().ok_or_else(|| {
                            Error::InvalidPdu("State event had no state key".to_owned())
                        })?),
                    ev.clone(),
                );
            } else {
                warn!(event_id = aid.borrow().as_str(), "missing auth event");
            }
        }

        stream::iter(
            auth_types
                .iter()
                .filter_map(|key| Some((key, resolved_state.get(key)?))),
        )
        .filter_map(|(key, ev_id)| async move {
            if let Some(event) = auth_events.get(ev_id.borrow()) {
                Some((key, event.clone()))
            } else {
                Some((key, fetch_event(ev_id.clone()).await?))
            }
        })
        .for_each(|(key, event)| {
            auth_state.insert(key.to_owned(), event);
            future::ready(())
        })
        .await;

        debug!("event to check {:?}", event.event_id());

        let current_third_party = auth_state.iter().find_map(|(_, pdu)| {
            (*pdu.event_type() == TimelineEventType::RoomThirdPartyInvite).then_some(pdu)
        });

        let fetch_state =
            |ty: &StateEventType, key: &str| future::ready(auth_state.get(&ty.with_state_key(key)));

        if auth_check(
            room_version,
            &event,
            current_third_party.as_ref(),
            fetch_state,
        )
        .await?
        {
            resolved_state.insert(
                event.event_type().with_state_key(state_key),
                event_id.clone(),
            );
        } else {
            warn!("event {event_id} failed the authentication check");
        }
    }

    Ok(resolved_state)
}
