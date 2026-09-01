use std::sync::Arc;

use futures::{Stream, StreamExt};
use phantom_core::{
    Result, err,
    stream::{ReadyExt, TryIgnore},
};
use phantom_database::{Deserialized, Json, Map};
use ruma::{
    CanonicalJsonObject, RoomId, UserId,
    events::{AnySyncEphemeralRoomEvent, receipt::ReceiptEvent},
    serde::Raw,
};

use crate::{Dep, server_state};

pub(super) struct Data {
    roomuserid_privateread: Arc<Map>,
    roomuserid_lastprivatereadupdate: Arc<Map>,
    readreceiptid_readreceipt: Arc<Map>,
    services: Services,
}

struct Services {
    server_state: Dep<server_state::Service>,
}

/// One receipt as sync sends it: whose it is, when it was set, and the event
/// itself.
pub type ReceiptItem<'a> = (&'a UserId, u64, Raw<AnySyncEphemeralRoomEvent>);

impl Data {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let db = &args.db;

        Self {
            roomuserid_privateread: db["roomuserid_privateread"].clone(),
            roomuserid_lastprivatereadupdate: db["roomuserid_lastprivatereadupdate"].clone(),
            readreceiptid_readreceipt: db["readreceiptid_readreceipt"].clone(),
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
            },
        }
    }

    /// Replaces a user's receipt in a room.
    ///
    /// The old entry is deleted rather than overwritten, because the key
    /// carries the counter the receipt was set at — that is what a sync pages
    /// through, so a new receipt is a new key and the old one would otherwise
    /// be sent again forever.
    pub(super) async fn readreceipt_update(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        event: &ReceiptEvent,
    ) -> Result {
        type Key<'a> = (&'a str, u64, &'a str);

        let last_possible_key = (room_id, u64::MAX);

        self.readreceiptid_readreceipt
            .rev_keys_from(&last_possible_key)
            .ignore_err()
            .ready_take_while(|(room, ..): &Key<'_>| *room == room_id.as_str())
            .ready_filter(|(_, _, user): &Key<'_>| *user == user_id.as_str())
            .ready_for_each(|key: Key<'_>| {
                self.readreceiptid_readreceipt.del(key).ok();
            })
            .await;

        let count = self.services.server_state.next_count()?;

        self.readreceiptid_readreceipt
            .put((room_id, count, user_id), Json(event))
    }

    /// The receipts set in a room after `since`.
    pub(super) fn readreceipts_since<'a>(
        &'a self,
        room_id: &'a RoomId,
        since: u64,
    ) -> impl Stream<Item = ReceiptItem<'a>> + Send + 'a {
        type Key<'a> = (&'a str, u64, &'a str);
        type KeyVal<'a> = (Key<'a>, CanonicalJsonObject);

        let after_since = since.saturating_add(1);
        let first_possible_edu = (room_id, after_since);

        self.readreceiptid_readreceipt
            .stream_from(&first_possible_edu)
            .ignore_err()
            .ready_take_while(move |((room, ..), _): &KeyVal<'_>| *room == room_id.as_str())
            .map(move |((_, count, user_id), mut json): KeyVal<'_>| {
                json.remove("room_id");

                let user_id = <&UserId>::try_from(user_id)
                    .map_err(|e| err!(Database("Invalid user id in read receipt key: {e}")))?;

                let event = serde_json::value::to_raw_value(&json)?;

                Ok((user_id, count, Raw::from_json(event)))
            })
            .ignore_err()
    }

    /// Moves a user's private read marker, and stamps when it moved.
    ///
    /// Two writes because they answer two questions: where the marker is, and
    /// whether it has moved since the token a sync is holding.
    pub(super) fn private_read_set(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        pdu_count: u64,
    ) -> Result {
        let key = (room_id, user_id);
        let next_count = self.services.server_state.next_count()?;

        self.roomuserid_privateread.put(key, pdu_count)?;
        self.roomuserid_lastprivatereadupdate.put(key, next_count)
    }

    pub(super) async fn private_read_get_count(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<u64> {
        self.roomuserid_privateread
            .qry(&(room_id, user_id))
            .await
            .deserialized()
    }

    pub(super) async fn last_privateread_update(&self, user_id: &UserId, room_id: &RoomId) -> u64 {
        self.roomuserid_lastprivatereadupdate
            .qry(&(room_id, user_id))
            .await
            .deserialized()
            .unwrap_or(0)
    }
}
