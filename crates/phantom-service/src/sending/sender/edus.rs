use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

use super::EDU_LIMIT;
use crate::sending::{EduBuf, EduVec, Service};
use futures::{StreamExt, future::OptionFuture, join, pin_mut};
use phantom_core::{
    Result, error,
    result::LogErr,
    stream::{BroadbandExt, ReadyExt},
    tracing,
};
use ruma::{
    OwnedRoomId, OwnedUserId, RoomId, ServerName,
    api::federation::transactions::edu::{
        DeviceListUpdateContent, Edu, PresenceContent, PresenceUpdate, ReceiptContent, ReceiptData,
        ReceiptMap,
    },
    device_id,
    events::{AnySyncEphemeralRoomEvent, receipt::ReceiptType},
    uint,
};

const SELECT_PRESENCE_LIMIT: usize = 256;
const SELECT_RECEIPT_LIMIT: usize = 256;
const SELECT_EDU_LIMIT: usize = EDU_LIMIT - 2;

impl Service {
    #[tracing::instrument(name = "edus", level = "debug", skip_all)]
    pub(super) async fn select_edus(&self, server_name: &ServerName) -> Result<(EduVec, u64)> {
        let since = self.db.get_latest_educount(server_name).await;
        let since_upper = self.services.server_state.current_count();
        let batch = (since, since_upper);
        debug_assert!(batch.0 <= batch.1, "since range must not be negative");

        let events_len = AtomicUsize::default();
        let max_edu_count = AtomicU64::new(since);

        let device_changes =
            self.select_edus_device_changes(server_name, batch, &max_edu_count, &events_len);

        let receipts: OptionFuture<_> = self
            .server
            .config
            .presence
            .allow_outgoing_read_receipts
            .then(|| self.select_edus_receipts(server_name, batch, &max_edu_count))
            .into();

        let presence: OptionFuture<_> = self
            .server
            .config
            .presence
            .allow_outgoing_presence
            .then(|| self.select_edus_presence(server_name, batch, &max_edu_count))
            .into();

        let (device_changes, receipts, presence) = join!(device_changes, receipts, presence);

        let mut events = device_changes;
        events.extend(presence.into_iter().flatten());
        events.extend(receipts.into_iter().flatten());

        Ok((events, max_edu_count.load(Ordering::Acquire)))
    }

    #[tracing::instrument(
        name = "device_changes",
        level = "trace",
        skip(self, server_name, max_edu_count)
    )]
    async fn select_edus_device_changes(
        &self,
        server_name: &ServerName,
        since: (u64, u64),
        max_edu_count: &AtomicU64,
        events_len: &AtomicUsize,
    ) -> EduVec {
        let mut events = EduVec::new();
        let server_rooms = self.services.state_cache.server_rooms(server_name);

        pin_mut!(server_rooms);
        let mut device_list_changes = HashSet::<OwnedUserId>::new();
        while let Some(room_id) = server_rooms.next().await {
            let keys_changed = self
                .services
                .users
                .room_keys_changed(room_id, since.0, None)
                .ready_filter(|(user_id, _)| self.services.server_state.user_is_local(user_id));

            pin_mut!(keys_changed);
            while let Some((user_id, count)) = keys_changed.next().await {
                if count > since.1 {
                    break;
                }

                max_edu_count.fetch_max(count, Ordering::Relaxed);
                if !device_list_changes.insert(user_id.into()) {
                    continue;
                }

                let edu = Edu::DeviceListUpdate(DeviceListUpdateContent::new(
                    user_id.into(),
                    device_id!("placeholder").to_owned(),
                    uint!(1),
                ));

                let mut buf = EduBuf::new();
                serde_json::to_writer(&mut buf, &edu)
                    .expect("failed to serialize device list update to JSON");

                events.push(buf);
                if events_len.fetch_add(1, Ordering::Relaxed) >= SELECT_EDU_LIMIT - 1 {
                    return events;
                }
            }
        }

        events
    }

    #[tracing::instrument(
        name = "receipts",
        level = "trace",
        skip(self, server_name, max_edu_count)
    )]
    async fn select_edus_receipts(
        &self,
        server_name: &ServerName,
        since: (u64, u64),
        max_edu_count: &AtomicU64,
    ) -> Option<EduBuf> {
        let num = &AtomicUsize::default();
        let receipts: BTreeMap<OwnedRoomId, ReceiptMap> = self
            .services
            .state_cache
            .server_rooms(server_name)
            .map(RoomId::to_owned)
            .broad_filter_map(|room_id| async move {
                let receipt_map = self
                    .select_edus_receipts_room(&room_id, since, max_edu_count, num)
                    .await;

                receipt_map
                    .read
                    .is_empty()
                    .eq(&false)
                    .then_some((room_id, receipt_map))
            })
            .collect()
            .await;

        if receipts.is_empty() {
            return None;
        }

        let receipt_content = Edu::Receipt(ReceiptContent::new(receipts));

        let mut buf = EduBuf::new();
        serde_json::to_writer(&mut buf, &receipt_content)
            .expect("Failed to serialize Receipt EDU to JSON vec");

        Some(buf)
    }

    #[tracing::instrument(name = "receipts", level = "trace", skip(self, since, max_edu_count))]
    async fn select_edus_receipts_room(
        &self,
        room_id: &RoomId,
        since: (u64, u64),
        max_edu_count: &AtomicU64,
        num: &AtomicUsize,
    ) -> ReceiptMap {
        let receipts = self
            .services
            .read_receipt
            .readreceipts_since(room_id, since.0);

        pin_mut!(receipts);
        let mut read = BTreeMap::<OwnedUserId, ReceiptData>::new();
        while let Some((user_id, count, read_receipt)) = receipts.next().await {
            if count > since.1 {
                break;
            }

            max_edu_count.fetch_max(count, Ordering::Relaxed);
            if !self.services.server_state.user_is_local(user_id) {
                continue;
            }

            let Ok(event) = serde_json::from_str(read_receipt.json().get()) else {
                error!(
                    ?user_id,
                    ?count,
                    ?read_receipt,
                    "Invalid edu event in read_receipts."
                );
                continue;
            };

            let AnySyncEphemeralRoomEvent::Receipt(r) = event else {
                error!(
                    ?user_id,
                    ?count,
                    ?event,
                    "Invalid event type in read_receipts"
                );
                continue;
            };

            let (event_id, mut receipt) = r
                .content
                .0
                .into_iter()
                .next()
                .expect("we only use one event per read receipt");

            let receipt = receipt
                .remove(&ReceiptType::Read)
                .expect("our read receipts always set this")
                .remove(user_id)
                .expect("our read receipts always have the user here");

            let receipt_data = ReceiptData::new(receipt, vec![event_id.clone()]);

            if read.insert(user_id.to_owned(), receipt_data).is_none()
                && num.fetch_add(1, Ordering::Relaxed) >= SELECT_RECEIPT_LIMIT - 1
            {
                break;
            }
        }

        ReceiptMap::new(read)
    }

    #[tracing::instrument(
        name = "presence",
        level = "trace",
        skip(self, server_name, max_edu_count)
    )]
    async fn select_edus_presence(
        &self,
        server_name: &ServerName,
        since: (u64, u64),
        max_edu_count: &AtomicU64,
    ) -> Option<EduBuf> {
        let presence_since = self.services.presence.presence_since(since.0);

        pin_mut!(presence_since);
        let mut presence_updates = HashMap::<OwnedUserId, PresenceUpdate>::new();
        while let Some((user_id, count, presence_bytes)) = presence_since.next().await {
            if count > since.1 {
                break;
            }

            max_edu_count.fetch_max(count, Ordering::Relaxed);
            if !self.services.server_state.user_is_local(user_id) {
                continue;
            }

            if !self
                .services
                .state_cache
                .server_sees_user(server_name, user_id)
                .await
            {
                continue;
            }

            let Ok(presence_event) = self
                .services
                .presence
                .from_json_bytes_to_event(presence_bytes, user_id)
                .await
                .log_err()
            else {
                continue;
            };

            let mut update = PresenceUpdate::new(
                user_id.into(),
                presence_event.content.presence,
                presence_event
                    .content
                    .last_active_ago
                    .unwrap_or_else(|| uint!(0)),
            );
            update.currently_active = presence_event.content.currently_active.unwrap_or(false);
            update.status_msg = presence_event.content.status_msg;

            presence_updates.insert(user_id.into(), update);
            if presence_updates.len() >= SELECT_PRESENCE_LIMIT {
                break;
            }
        }

        if presence_updates.is_empty() {
            return None;
        }

        let presence_content = Edu::Presence(PresenceContent::new(
            presence_updates.into_values().collect(),
        ));

        let mut buf = EduBuf::new();
        serde_json::to_writer(&mut buf, &presence_content)
            .expect("failed to serialize Presence EDU to JSON");

        Some(buf)
    }
}
