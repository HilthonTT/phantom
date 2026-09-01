//! How far each user has read in a room.
//!
//! Two markers, kept apart because they are told to different people. A public
//! receipt is an event everyone in the room sees, and is federated; a private
//! one is only ever sent back to the user who set it, and never leaves this
//! server.
//!
//! Both are stored as the counter they were set at rather than as an event id,
//! so a sync can ask what moved since the token it holds without reading the
//! receipts themselves.

mod data;

use std::{collections::BTreeMap, sync::Arc};

use futures::{Stream, TryFutureExt, try_join};
use phantom_core::{
    Result, debug, err, implement,
    matrix::pdu::{PduCount, PduId, RawPduId},
    trace, warn,
};
use ruma::{
    OwnedEventId, OwnedUserId, RoomId, UserId,
    events::{
        AnySyncEphemeralRoomEvent, SyncEphemeralRoomEvent,
        receipt::{Receipt, ReceiptEvent, ReceiptEventContent, ReceiptType, Receipts},
    },
    serde::Raw,
};

use self::data::Data;
pub use self::data::ReceiptItem;
use crate::{Dep, rooms};

pub struct Service {
    services: Services,
    db: Data,
}

struct Services {
    short: Dep<rooms::short::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
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

/// Replaces a user's public read receipt in a room.
#[implement(Service)]
pub async fn readreceipt_update(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    event: &ReceiptEvent,
) -> Result {
    self.db.readreceipt_update(user_id, room_id, event).await

    // Upstream then flushes the room's outbound queue, so the receipt reaches
    // the other servers now rather than with whatever is sent next. phantom
    // has no sending service yet; that call belongs here when it does.
}

/// The user's own private read receipt, as a sync event.
///
/// Built here rather than stored: what is kept is the counter the marker sits
/// at, and the event a client expects names the PDU at that counter.
#[implement(Service)]
pub async fn private_read_get(
    &self,
    room_id: &RoomId,
    user_id: &UserId,
) -> Result<Raw<AnySyncEphemeralRoomEvent>> {
    let pdu_count = self.private_read_get_count(room_id, user_id).map_err(|e| {
        err!(Database(warn!(
            "No private read receipt was set in {room_id}: {e}"
        )))
    });

    let shortroomid = self.services.short.get_shortroomid(room_id).map_err(|e| {
        err!(Database(warn!(
            "Short room ID does not exist in database for {room_id}: {e}"
        )))
    });

    let (pdu_count, shortroomid) = try_join!(pdu_count, shortroomid)?;

    let pdu_id: RawPduId = PduId {
        shortroomid,
        shorteventid: PduCount::Normal(pdu_count),
    }
    .into();

    let pdu = self.services.timeline.get_pdu_from_id(&pdu_id).await?;

    let event_id: OwnedEventId = pdu.event_id;
    let user_id: OwnedUserId = user_id.to_owned();

    // The default is an unthreaded receipt with no timestamp, which is what
    // this is: the timestamp is not stored, and a receipt without one is
    // valid where inventing one would not be.
    let receipt = Receipt::default();

    let content = ReceiptEventContent(BTreeMap::from_iter([(
        event_id,
        Receipts::from_iter([(
            ReceiptType::ReadPrivate,
            BTreeMap::from_iter([(user_id, receipt)]),
        )]),
    )]));

    let event = serde_json::value::to_raw_value(&SyncEphemeralRoomEvent::new(content))
        .expect("receipt created manually");

    Ok(Raw::from_json(event))
}

/// The receipts set in a room after `since`.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub fn readreceipts_since<'a>(
    &'a self,
    room_id: &'a RoomId,
    since: u64,
) -> impl Stream<Item = ReceiptItem<'a>> + Send + 'a {
    self.db.readreceipts_since(room_id, since)
}

/// Moves a user's private read marker to PDU `count`.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub fn private_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) -> Result {
    self.db.private_read_set(room_id, user_id, count)
}

/// Where a user's private read marker sits.
#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn private_read_get_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
    self.db.private_read_get_count(room_id, user_id).await
}

/// The counter the user's private marker last moved at, or zero where it never
/// has — which is before any real counter, so a sync sees it as unchanged.
#[implement(Service)]
#[inline]
pub async fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    self.db.last_privateread_update(user_id, room_id).await
}

/// Folds many receipt events into the single one a sync response carries.
///
/// A receipt that will not parse is dropped rather than failing the batch: it
/// is one user's read marker, and losing the rest of the room's over it would
/// be the worse outcome.
#[must_use]
pub fn pack_receipts<I>(receipts: I) -> Raw<SyncEphemeralRoomEvent<ReceiptEventContent>>
where
    I: Iterator<Item = Raw<AnySyncEphemeralRoomEvent>>,
{
    let mut json = BTreeMap::new();

    for value in receipts {
        match serde_json::from_str::<SyncEphemeralRoomEvent<ReceiptEventContent>>(
            value.json().get(),
        ) {
            Ok(value) => json.extend(value.content),
            Err(e) => debug!("failed to parse receipt: {e}"),
        }
    }

    let content = ReceiptEventContent::from_iter(json);
    trace!(?content);

    Raw::from_json(
        serde_json::value::to_raw_value(&SyncEphemeralRoomEvent::new(content))
            .expect("received valid json"),
    )
}
