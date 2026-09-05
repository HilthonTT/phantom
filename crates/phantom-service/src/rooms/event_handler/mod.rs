//! Taking an event another server sent us and deciding what it means.
//!
//! This is the hard half of federation. A server hands us an event and claims
//! it belongs in a room; everything about that claim has to be checked, and
//! most of the checking needs events we do not have and have to go and ask
//! for. The work divides into five questions, one per submodule:
//!
//! 1. **Is the event what it says it is?** Its id is the hash of its content,
//!    so it is recomputed rather than trusted, and its signatures are checked
//!    against the keys of the servers that claim to have signed it. That, plus
//!    checking it against its own `auth_events`, is [`outliers`] — an outlier
//!    being an event we believe but cannot yet place.
//! 2. **May that server speak here at all?** [`acl`], against the room's
//!    `m.room.server_acl`.
//! 3. **What came before it?** An event names its predecessors, and we may
//!    have none of them. [`prev`] fetches the gap and handles it oldest first,
//!    under a budget, because a server that has been offline for a week can
//!    hand us a gap we would spend the rest of the day filling.
//! 4. **What was the room's state when it happened?** [`state`] — from our own
//!    record where the predecessors are known, by resolving their states where
//!    they disagree, and by asking the sending server where we know nothing.
//! 5. **Is it allowed?** [`upgrade`] authorizes the event against the state at
//!    it, decides whether it also passes against the room's *current* state —
//!    an event that fails only the second is soft-failed rather than rejected
//!    — resolves the room's new state, and appends it.
//!
//! Two things are shared across all of that and live here.
//!
//! [`mutex_federation`] serializes handling per room. Two events arriving at
//! once for the same room would each resolve state against a room the other
//! is in the middle of changing, and the loser's work would be wasted at best.
//!
//! [`bad_events`] remembers events that could not be fetched or would not
//! validate. Without it a room with one unreachable predecessor re-asks for it
//! on every event that references it, which is every event in the room.
//!
//! [`mutex_federation`]: Service::mutex_federation
//! [`bad_events`]: Service::bad_events

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
    /// Held for as long as one event is being handled for a room.
    pub mutex_federation: RoomMutexMap,

    /// Events that would not fetch or would not validate, and how many times
    /// we have tried, so a retry can be held off exponentially.
    pub bad_events: RwLock<HashMap<OwnedEventId, (Instant, u32)>>,

    /// How long [`prev`] may spend filling a gap before it gives up on the
    /// rest of it.
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

/// The shortest a bad event is held off for, doubling with each further
/// failure up to [`MAX_BACKOFF`].
const MIN_BACKOFF: u64 = 60;

/// The longest a bad event is held off for. An event still unfetchable after
/// this long is very likely gone for good, but the room is not, so retrying
/// once an hour costs nothing and recovers from an outage that has ended.
const MAX_BACKOFF: u64 = 60 * 60;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let budget = args.server.config.federation_prev_event_budget_s;

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

/// Handles an event that arrived from `origin`, putting it in the room where
/// it belongs.
///
/// `Some(id)` means this call put the event in the timeline. `None` means it
/// did not, which covers three quite different things — the event was already
/// there, it was soft-failed, or it is older than anything this server holds
/// for the room — and none of them is an error: all three end with the room in
/// a consistent state and nothing for the sender to do.
///
/// `is_timeline_event` says whether the event was sent to us as part of the
/// room's timeline, rather than pulled in as somebody's predecessor or auth
/// event. Only a timeline event is worth filling a gap for.
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
    // Already in the timeline: nothing to do, and in particular nothing to
    // fetch. A server resending a transaction is the ordinary way here.
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

    // Two different refusals, asked in the order of whose decision it is. The
    // room's ACL is the room's business and applies to every server in it; the
    // moderation list is this operator's, and applies whatever the room says.
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

    // The create event decides the room version, and the room version decides
    // how everything below reads the event. A room without one is a room this
    // server should not have a record of.
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

    // An event older than anything this server holds for the room would need
    // the whole history before it to be placed, which is backfill's job and
    // not something to do in the middle of receiving a transaction.
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

/// Whether an event should be left alone for now because the last attempt at
/// it failed recently.
#[implement(Service)]
fn is_backed_off(&self, event_id: &EventId) -> bool {
    let bad = self.bad_events.read().expect("locked");

    bad.get(event_id).is_some_and(|(last, tries)| {
        continue_exponential_backoff_secs(MIN_BACKOFF, MAX_BACKOFF, last.elapsed(), *tries)
    })
}

/// Records that an event could not be obtained or would not validate.
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

/// Forgets that an event was bad, once it has been obtained after all.
#[implement(Service)]
fn mark_good(&self, event_id: &EventId) {
    self.bad_events.write().expect("locked").remove(event_id);
}

/// Whether the event was accepted by the room but withheld from it.
#[implement(Service)]
pub async fn is_soft_failed(&self, event_id: &EventId) -> bool {
    self.db.softfailedeventids.get(event_id).await.is_ok()
}

#[implement(Service)]
fn mark_soft_failed(&self, event_id: &EventId) {
    self.db.softfailedeventids.insert(event_id, []).ok();
}
