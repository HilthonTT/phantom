//! What the server records about an event besides the event itself.
//!
//! Three columns, each answering something the timeline cannot. Which events
//! relate to a given one, so an edit or a reaction can be found from what it
//! points at. Which events have already been named as a `prev_event`, which is
//! how a room's forward extremities are worked out. And which events were
//! soft-failed, so one arriving a second time is not reconsidered.
//!
//! A relation is stored as a pair of short ids, target first, with nothing in
//! the value. That is what lets an event's relations be read as a single scan
//! from a prefix, in either direction, without touching the events themselves
//! until a caller asks for them.
//!
//! `get_relations` returns that chain as it stands. The spec's `/relations` is
//! more than this: it also filters by `rel_type` and by event type, and drops
//! the events the requesting user may not see. That last part waits on
//! `rooms::state_accessor`, so the filtering is not here yet.

mod data;

use std::sync::Arc;

use futures::{StreamExt, future::try_join};
use phantom_core::{Result, implement, matrix::pdu::PduCount};
use ruma::{EventId, RoomId, UserId, api::Direction};

use self::data::Data;
use crate::{Dep, rooms, rooms::timeline::PdusIterItem};

pub struct Service {
    services: Services,
    db: Data,
}

struct Services {
    short: Dep<rooms::short::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
            db: Data::new(&args),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Records that the event counted `from` relates to the event counted `to`.
///
/// Only a pair of forward-timeline counts is recorded. A backfilled count is
/// negative where the column is keyed on unsigned ids, so a relation involving
/// one is dropped rather than stored under a key that would collide with a
/// forward event's.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn add_relation(&self, from: PduCount, to: PduCount) {
    if let (PduCount::Normal(from), PduCount::Normal(to)) = (from, to) {
        self.db.add_relation(from, to);
    }
}

/// The events relating to `target`, walking from `from` in `dir` order.
///
/// A relation of a relation is followed up to `max_depth`, which is what an
/// edited reply or a thread of reactions needs to come back in one response.
/// At most `limit` events are returned however deep the walk goes.
#[implement(Service)]
#[allow(clippy::too_many_arguments)]
pub async fn get_relations(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    target: &EventId,
    from: PduCount,
    limit: usize,
    max_depth: u8,
    dir: Direction,
) -> Vec<PdusIterItem> {
    let room_id = self.services.short.get_shortroomid(room_id);
    let target = self.services.timeline.get_pdu_count(target);

    let Ok((room_id, target)) = try_join(room_id, target).await else {
        return Vec::new();
    };

    let mut pdus: Vec<_> = self
        .db
        .get_relations(user_id, room_id, unsigned_count(target), from, dir)
        .take(limit)
        .collect()
        .await;

    let mut stack: Vec<_> = pdus
        .iter()
        .filter(|_| max_depth > 0)
        .map(|pdu| (pdu.clone(), 1))
        .collect();

    'limit: while let Some((stack_pdu, depth)) = stack.pop() {
        let relations: Vec<_> = self
            .db
            .get_relations(user_id, room_id, unsigned_count(stack_pdu.0), from, dir)
            .collect()
            .await;

        for relation in relations {
            if depth < max_depth {
                stack.push((relation.clone(), depth.saturating_add(1)));
            }

            pdus.push(relation);
            if pdus.len() >= limit {
                break 'limit;
            }
        }
    }

    pdus
}

/// The count to scan a relation chain from.
///
/// A backfilled count is negative and the relation column is keyed on unsigned
/// ids, so there is nothing to scan for one: zero is returned, which reads as
/// an empty chain rather than as the relations of some other event.
fn unsigned_count(count: PduCount) -> u64 {
    match count {
        PduCount::Normal(count) => count,
        PduCount::Backfilled(_) => 0,
    }
}

/// Records every event in `event_ids` as referenced by one in `room_id`.
#[implement(Service)]
#[tracing::instrument(skip_all, level = "debug")]
pub fn mark_as_referenced<'a, I>(&self, room_id: &RoomId, event_ids: I)
where
    I: Iterator<Item = &'a EventId>,
{
    self.db.mark_as_referenced(room_id, event_ids);
}

/// Whether any event in `room_id` names `event_id` as a `prev_event`.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> bool {
    self.db.is_event_referenced(room_id, event_id).await
}

/// Records the event as soft-failed.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub fn mark_event_soft_failed(&self, event_id: &EventId) {
    self.db.mark_event_soft_failed(event_id);
}

/// Whether the event was soft-failed when it was first seen.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn is_event_soft_failed(&self, event_id: &EventId) -> bool {
    self.db.is_event_soft_failed(event_id).await
}
