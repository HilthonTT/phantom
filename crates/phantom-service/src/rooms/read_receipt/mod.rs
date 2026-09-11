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

#[implement(Service)]
pub async fn readreceipt_update(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    event: &ReceiptEvent,
) -> Result {
    self.db.readreceipt_update(user_id, room_id, event).await
}

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

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub fn private_read_set(&self, room_id: &RoomId, user_id: &UserId, count: u64) -> Result {
    self.db.private_read_set(room_id, user_id, count)
}

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn private_read_get_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
    self.db.private_read_get_count(room_id, user_id).await
}

#[implement(Service)]
#[inline]
pub async fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
    self.db.last_privateread_update(user_id, room_id).await
}

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

#[implement(Service)]
#[inline]
#[tracing::instrument(skip(self), level = "debug")]
pub(super) async fn delete_all_read_receipts(&self, room_id: &RoomId) {
    self.db.delete_all_read_receipts(room_id).await;
}
