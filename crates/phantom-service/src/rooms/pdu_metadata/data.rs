use std::sync::Arc;

use arrayvec::ArrayVec;
use futures::{Stream, StreamExt};
use phantom_core::{
    bytes::u64_from_u8,
    matrix::pdu::{PduCount, PduId, RawPduId},
    result::LogErr,
    stream::{ReadyExt, TryIgnore, WidebandExt},
};
use phantom_database::Map;
use ruma::{EventId, RoomId, UserId, api::Direction};

use crate::{
    Dep, rooms,
    rooms::{
        short::{ShortEventId, ShortRoomId},
        timeline::PdusIterItem,
    },
};

pub(super) struct Data {
    tofrom_relation: Arc<Map>,
    referencedevents: Arc<Map>,
    softfailedeventids: Arc<Map>,
    services: Services,
}

struct Services {
    timeline: Dep<rooms::timeline::Service>,
}

impl Data {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let db = &args.db;
        Self {
            tofrom_relation: db["tofrom_relation"].clone(),
            referencedevents: db["referencedevents"].clone(),
            softfailedeventids: db["softfailedeventids"].clone(),
            services: Services {
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
        }
    }

    /// Records the relation, keyed target first so that an event's relations
    /// sit together under one prefix.
    pub(super) fn add_relation(&self, from: u64, to: u64) {
        const BUFSIZE: usize = size_of::<u64>() * 2;

        let key: &[u64] = &[to, from];
        self.tofrom_relation.aput_raw::<BUFSIZE, _, _>(key, []).ok();
    }

    /// The events relating to `target`, as one scan of that prefix.
    ///
    /// The transaction id is stripped from every event that is not the
    /// requesting user's own, since it is only ever meaningful to its sender.
    pub(super) fn get_relations<'a>(
        &'a self,
        user_id: &'a UserId,
        shortroomid: ShortRoomId,
        target: ShortEventId,
        from: PduCount,
        dir: Direction,
    ) -> impl Stream<Item = PdusIterItem> + Send + 'a {
        let mut current = ArrayVec::<u8, 16>::new();
        current.extend(target.to_be_bytes());
        current.extend(from.saturating_inc(dir).into_unsigned().to_be_bytes());
        let current = current.as_slice();

        match dir {
            Direction::Forward => self.tofrom_relation.raw_keys_from(current).boxed(),
            Direction::Backward => self.tofrom_relation.rev_raw_keys_from(current).boxed(),
        }
        .ignore_err()
        .ready_take_while(move |key| key.starts_with(&target.to_be_bytes()))
        .map(|to_from| u64_from_u8(&to_from[8..16]))
        .map(PduCount::from_unsigned)
        .wide_filter_map(move |shorteventid| async move {
            let pdu_id: RawPduId = PduId {
                shortroomid,
                shorteventid,
            }
            .into();

            let mut pdu = self.services.timeline.get_pdu_from_id(&pdu_id).await.ok()?;

            if pdu.sender != user_id {
                pdu.remove_transaction_id().log_err().ok();
            }

            Some((shorteventid, pdu))
        })
    }

    /// Records every event in `event_ids` as referenced by one in `room_id`.
    #[inline]
    pub(super) fn mark_as_referenced<'a, I>(&self, room_id: &RoomId, event_ids: I)
    where
        I: Iterator<Item = &'a EventId>,
    {
        for prev in event_ids {
            let key = (room_id, prev);
            self.referencedevents.put_raw(key, []).ok();
        }
    }

    /// Whether any event in `room_id` names `event_id` as a `prev_event`.
    pub(super) async fn is_event_referenced(&self, room_id: &RoomId, event_id: &EventId) -> bool {
        let key = (room_id, event_id);
        self.referencedevents.qry(&key).await.is_ok()
    }

    /// Records the event as soft-failed.
    pub(super) fn mark_event_soft_failed(&self, event_id: &EventId) {
        self.softfailedeventids.insert(event_id, []).ok();
    }

    /// Whether the event was soft-failed when it was first seen.
    pub(super) async fn is_event_soft_failed(&self, event_id: &EventId) -> bool {
        self.softfailedeventids.get(event_id).await.is_ok()
    }
}
