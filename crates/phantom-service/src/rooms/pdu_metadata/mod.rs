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

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn add_relation(&self, from: PduCount, to: PduCount) {
    if let (PduCount::Normal(from), PduCount::Normal(to)) = (from, to) {
        self.db.add_relation(from, to);
    }
}

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

fn unsigned_count(count: PduCount) -> u64 {
    match count {
        PduCount::Normal(count) => count,
        PduCount::Backfilled(_) => 0,
    }
}

#[implement(Service)]
#[tracing::instrument(skip_all, level = "debug")]
pub fn mark_as_referenced<'a, I>(&self, room_id: &RoomId, event_ids: I)
where
    I: Iterator<Item = &'a EventId>,
{
    self.db.mark_as_referenced(room_id, event_ids);
}

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> bool {
    self.db.is_event_referenced(room_id, event_id).await
}

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub fn mark_event_soft_failed(&self, event_id: &EventId) {
    self.db.mark_event_soft_failed(event_id);
}

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn is_event_soft_failed(&self, event_id: &EventId) -> bool {
    self.db.is_event_soft_failed(event_id).await
}

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub(super) async fn delete_all_referenced(&self, room_id: &RoomId) {
    self.db.delete_all_referenced(room_id).await;
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "trace")]
pub(super) async fn purge_event(&self, shorteventid: &[u8], event_id: &EventId) {
    self.db.purge_relations(shorteventid).await;
    self.db.purge_soft_failed(event_id);
}
