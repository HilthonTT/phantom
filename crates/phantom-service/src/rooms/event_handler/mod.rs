mod acl;
mod outliers;
mod parse;
mod prev;
mod state;
mod upgrade;

use std::{
    collections::HashMap,
    fmt::Write,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use phantom_core::{
    Err, Result, debug, err, implement, matrix::pdu::RawPduId, sync::MutexMap,
    time::exponential_backoff::continue_exponential_backoff_secs,
};
use phantom_database::Map;
use ruma::{CanonicalJsonObject, EventId, OwnedEventId, OwnedRoomId, RoomId, ServerName};

use crate::{
    Dep,
    moderation::{self, Restriction},
    rooms, server_keys, server_state,
};

pub struct Service {
    pub mutex_federation: RoomMutexMap,

    pub bad_events: RwLock<HashMap<OwnedEventId, (Instant, u32)>>,

    prev_event_budget: Duration,

    services: Services,
    db: Data,
}

struct Services {
    auth_chain: Dep<rooms::auth_chain::Service>,
    federation: Dep<crate::federation::Service>,
    metadata: Dep<rooms::metadata::Service>,
    moderation: Dep<moderation::Service>,
    outlier: Dep<rooms::outlier::Service>,
    server_keys: Dep<server_keys::Service>,
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_compressor: Dep<rooms::state_compressor::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

struct Data {
    softfailedeventids: Arc<Map>,
}

type RoomMutexMap = MutexMap<OwnedRoomId, ()>;

const MIN_BACKOFF: u64 = 60;

const MAX_BACKOFF: u64 = 60 * 60;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let budget = args.server.config.federation.federation_prev_event_budget_s;

        Ok(Arc::new(Self {
            mutex_federation: RoomMutexMap::new(),
            bad_events: RwLock::new(HashMap::new()),
            prev_event_budget: Duration::from_secs(budget),
            services: Services {
                auth_chain: args.depend::<rooms::auth_chain::Service>("rooms::auth_chain"),
                federation: args.depend::<crate::federation::Service>("federation"),
                metadata: args.depend::<rooms::metadata::Service>("rooms::metadata"),
                moderation: args.depend::<moderation::Service>("moderation"),
                outlier: args.depend::<rooms::outlier::Service>("rooms::outlier"),
                server_keys: args.depend::<server_keys::Service>("server_keys"),
                server_state: args.depend::<server_state::Service>("server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_compressor: args
                    .depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
            db: Data {
                softfailedeventids: args.db["softfailedeventids"].clone(),
            },
        }))
    }

    async fn clear_cache(&self) {
        self.bad_events.write().expect("locked").clear();
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        writeln!(out, "federation_mutex: {}", self.mutex_federation.len())?;
        writeln!(
            out,
            "bad_events: {}",
            self.bad_events.read().expect("locked").len()
        )?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
#[tracing::instrument(name = "handle", level = "info", skip_all, fields(%origin, %room_id, %event_id))]
pub async fn handle_incoming_pdu(
    &self,
    origin: &ServerName,
    room_id: &RoomId,
    event_id: &EventId,
    value: CanonicalJsonObject,
    is_timeline_event: bool,
) -> Result<Option<RawPduId>> {
    if self.services.timeline.pdu_exists(event_id).await {
        debug!("Event is already in the timeline");
        return Ok(None);
    }

    if !self.services.metadata.exists(room_id).await {
        return Err!(Request(NotFound("Room is unknown to this server.")));
    }

    if self.services.metadata.is_disabled(room_id).await {
        return Err!(Request(Forbidden(
            "Federation of this room is currently disabled on this server."
        )));
    }

    self.acl_check(origin, room_id).await?;

    if self
        .services
        .moderation
        .forbids(origin, Restriction::Federation)
    {
        return Err!(Request(Forbidden(
            "This server does not federate with the server the event came from."
        )));
    }

    let create_event = self
        .services
        .state_accessor
        .room_state_get(room_id, &ruma::events::StateEventType::RoomCreate, "")
        .await
        .map_err(|e| err!(Database("The room has no m.room.create: {e}")))?;

    let room_version_id = self.services.state.get_room_version(room_id).await?;

    let first_ts_in_room = self
        .services
        .timeline
        .first_pdu_in_room(room_id)
        .await
        .map(|pdu| pdu.origin_server_ts)?;

    let _mutex = self.mutex_federation.lock(room_id).await;

    let (incoming_pdu, value) = self
        .handle_outlier_pdu(origin, &create_event, event_id, room_id, value, false)
        .await?;

    if is_timeline_event && incoming_pdu.origin_server_ts < first_ts_in_room {
        debug!("Event predates the room's history on this server");
        return Ok(None);
    }

    self.fill_gap(
        origin,
        &create_event,
        room_id,
        &room_version_id,
        &incoming_pdu,
        first_ts_in_room,
    )
    .await;

    self.upgrade_outlier_to_timeline_pdu(incoming_pdu, value, &create_event, origin, room_id)
        .await
}

#[implement(Service)]
fn is_backed_off(&self, event_id: &EventId) -> bool {
    let bad = self.bad_events.read().expect("locked");

    bad.get(event_id).is_some_and(|(last, tries)| {
        continue_exponential_backoff_secs(MIN_BACKOFF, MAX_BACKOFF, last.elapsed(), *tries)
    })
}

#[implement(Service)]
fn mark_bad(&self, event_id: &EventId) {
    self.bad_events
        .write()
        .expect("locked")
        .entry(event_id.to_owned())
        .and_modify(|(last, tries)| {
            *last = Instant::now();
            *tries = tries.saturating_add(1);
        })
        .or_insert((Instant::now(), 1));
}

#[implement(Service)]
fn mark_good(&self, event_id: &EventId) {
    self.bad_events.write().expect("locked").remove(event_id);
}

#[implement(Service)]
pub async fn is_soft_failed(&self, event_id: &EventId) -> bool {
    self.db.softfailedeventids.get(event_id).await.is_ok()
}

#[implement(Service)]
fn mark_soft_failed(&self, event_id: &EventId) {
    self.db.softfailedeventids.insert(event_id, []).ok();
}
