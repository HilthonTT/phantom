use std::{collections::BTreeMap, mem::take};

use futures::{FutureExt, StreamExt, TryFutureExt, future::try_join4};
use phantom_core::{
    Error, Result, error, extract_variant, implement,
    stream::{BroadbandExt, IterStream},
};
use phantom_service::accounts::account_data::AnyRawAccountDataEvent;
use phantom_service::rooms::read_receipt::pack_receipts;
use ruma::{
    OwnedRoomId, OwnedUserId, RoomId,
    api::client::sync::sync_events::v5::response,
    events::{
        AnyRoomAccountDataEvent, AnySyncEphemeralRoomEvent, GlobalAccountDataEventType,
        ignored_user_list::IgnoredUserListEventContent, receipt::SyncReceiptEvent,
    },
    serde::Raw,
};
use tokio::sync::OnceCell;

use super::{
    SyncInfo, Window, WindowRoom,
    connection::{Connection, Room},
    rooms::{
        Failure as RoomFailure,
        Failure::{Payload as PayloadFailure, Timeline as TimelineFailure},
        RoomDetails, handle_room, merged_room_details, room_config,
    },
};
use crate::client::account_data::is_empty_account_data_event;

type IgnoredUsers = Option<IgnoredUserListEventContent>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Domain {
    Timeline,
    Payload,
    PublicReceipt,
    PrivateRead,
    RoomAccountData,
}

#[derive(Debug)]
struct Failure {
    domain: Domain,
    error: Error,
}

impl Failure {
    fn new(domain: Domain, error: Error) -> Self {
        Self { domain, error }
    }
}

impl From<RoomFailure> for Failure {
    fn from(failure: RoomFailure) -> Self {
        match failure {
            TimelineFailure(error) => Self::new(Domain::Timeline, error),
            PayloadFailure(error) => Self::new(Domain::Payload, error),
        }
    }
}

#[derive(Debug)]
struct CompleteRange {
    payload: Option<response::Room>,
    config: Option<u64>,
    receipts: Option<Raw<SyncReceiptEvent>>,
    account_data: Vec<Raw<AnyRoomAccountDataEvent>>,
}

#[derive(Default)]
pub(super) struct Results {
    ranges: BTreeMap<OwnedRoomId, CompleteRange>,
}

#[implement(Results)]
pub(super) fn room_updates(&mut self) -> impl Iterator<Item = (&RoomId, Option<u64>)> + Send {
    self.ranges
        .iter_mut()
        .map(|(room_id, range)| (room_id.as_ref(), range.config.take()))
}

#[implement(Results)]
pub(super) fn payload(&self, room_id: &RoomId) -> Option<&response::Room> {
    self.ranges
        .get(room_id)
        .and_then(|range| range.payload.as_ref())
}

#[implement(Results)]
pub(super) fn into_payloads(self) -> BTreeMap<OwnedRoomId, response::Room> {
    self.ranges
        .into_iter()
        .filter_map(|(room_id, range)| range.payload.map(|payload| (room_id, payload)))
        .collect()
}

#[implement(Results)]
pub(super) fn take_receipts(&mut self, room_id: &RoomId) -> Option<Raw<SyncReceiptEvent>> {
    self.ranges
        .get_mut(room_id)
        .and_then(|range| range.receipts.take())
}

#[implement(Results)]
pub(super) fn take_account_data(
    &mut self,
    room_id: &RoomId,
) -> Option<Vec<Raw<AnyRoomAccountDataEvent>>> {
    self.ranges
        .get_mut(room_id)
        .map(|range| take(&mut range.account_data))
        .filter(|events| !events.is_empty())
}

#[tracing::instrument(
    name = "ranges",
    level = "debug",
    skip_all,
    fields(
        next_batch = conn.next_batch,
        window = window.len(),
    ),
)]
pub(super) async fn collect(
    sync_info: SyncInfo<'_>,
    conn: &Connection,
    window: &Window,
) -> Results {
    let ignored = OnceCell::new();
    let empty_room = Room::default();
    let ranges = window
        .iter()
        .stream()
        .broad_filter_map(async |(room_id, window_room)| {
            let room = conn.rooms.get(room_id).unwrap_or(&empty_room);

            let room_details = merged_room_details(conn, &window_room.lists, room_id);

            match collect_room(sync_info, conn, window_room, room, room_details, &ignored).await {
                Ok(range) => Some((room_id.clone(), range)),
                Err(Failure { domain, error }) => {
                    error!(
                        %room_id,
                        ?domain,
                        roomsince = room.roomsince,
                        next_batch = conn.next_batch,
                        %error,
                        "sliding sync range failed"
                    );
                    None
                }
            }
        })
        .collect()
        .await;

    Results { ranges }
}

async fn collect_room(
    sync_info: SyncInfo<'_>,
    conn: &Connection,
    window_room: &WindowRoom,
    room: &Room,
    room_details: RoomDetails,
    ignored: &OnceCell<IgnoredUsers>,
) -> Result<CompleteRange, Failure> {
    let room_id = &window_room.room_id;
    let config_hash = room_config(&room_details);
    let config_changed = room.config_hash != config_hash;
    let payload_is_fresh = window_room.payload_is_fresh(room.roomsince) || config_changed;

    let payload = async {
        if !payload_is_fresh {
            return Ok(None);
        }

        handle_room(
            sync_info,
            conn,
            window_room,
            room,
            config_changed,
            room_details,
        )
        .await
        .map(Some)
    }
    .map_err(Failure::from);

    let public_receipts = public_receipts(sync_info, conn, room_id, room.roomsince, ignored)
        .map_err(|error| Failure::new(Domain::PublicReceipt, error));

    let private_receipts = private_receipts(sync_info, conn, room_id, room.roomsince)
        .map_err(|error| Failure::new(Domain::PrivateRead, error));

    let account_data = room_account_data(sync_info, conn, room_id, room.roomsince)
        .map_err(|error| Failure::new(Domain::RoomAccountData, error));

    let (payload, public_receipts, private_receipts, account_data) =
        try_join4(payload, public_receipts, private_receipts, account_data)
            .boxed()
            .await?;

    Ok(assemble(
        payload,
        public_receipts,
        private_receipts,
        account_data,
        config_hash,
    ))
}

async fn public_receipts(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    room_id: &RoomId,
    roomsince: u64,
    ignored: &OnceCell<IgnoredUsers>,
) -> Result<impl Iterator<Item = Raw<AnySyncEphemeralRoomEvent>>> {
    let next_batch = conn.next_batch;
    let mut receipts: Vec<(OwnedUserId, Raw<AnySyncEphemeralRoomEvent>)> = services
        .rooms
        .read_receipt
        .readreceipts_since(room_id, roomsince)
        .filter(|(_, count, _)| futures::future::ready(*count <= next_batch))
        .map(|(user_id, _count, event)| (user_id.to_owned(), event))
        .collect()
        .await;

    if !receipts.is_empty() {
        let ignored = ignored
            .get_or_init(async || {
                services
                    .account_data
                    .get_global(sender_user, GlobalAccountDataEventType::IgnoredUserList)
                    .await
                    .ok()
            })
            .await;

        if let Some(ignored) = ignored {
            receipts.retain(|(user_id, _)| !ignored.ignored_users.contains_key(user_id));
        }
    }

    Ok(receipts.into_iter().map(|(_, event)| event))
}

async fn private_receipts(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    room_id: &RoomId,
    roomsince: u64,
) -> Result<Option<Raw<AnySyncEphemeralRoomEvent>>> {
    let read_receipt = &services.rooms.read_receipt;
    let update = read_receipt
        .last_privateread_update(sender_user, room_id)
        .await;

    // A private read past the bounded range is delivered by the next pass.
    if update <= roomsince || update > conn.next_batch {
        return Ok(None);
    }

    Ok(read_receipt
        .private_read_get(room_id, sender_user)
        .await
        .ok())
}

async fn room_account_data(
    SyncInfo {
        services,
        sender_user,
        ..
    }: SyncInfo<'_>,
    conn: &Connection,
    room_id: &RoomId,
    roomsince: u64,
) -> Result<Vec<Raw<AnyRoomAccountDataEvent>>> {
    Ok(services
        .account_data
        .changes_since(Some(room_id), sender_user, roomsince, Some(conn.next_batch))
        .filter_map(|event| {
            futures::future::ready(extract_variant!(event, AnyRawAccountDataEvent::Room))
        })
        .filter(move |event| {
            futures::future::ready(roomsince != 0 || !is_empty_account_data_event(event))
        })
        .collect()
        .await)
}

fn assemble<PublicReceipts>(
    payload: Option<response::Room>,
    public_receipts: PublicReceipts,
    private_receipt: Option<Raw<AnySyncEphemeralRoomEvent>>,
    account_data: Vec<Raw<AnyRoomAccountDataEvent>>,
    config: u64,
) -> CompleteRange
where
    PublicReceipts: Iterator<Item = Raw<AnySyncEphemeralRoomEvent>>,
{
    let mut receipts = public_receipts.chain(private_receipt).peekable();
    let receipts = receipts.peek().is_some().then(|| pack_receipts(receipts));

    let config = payload.as_ref().map(|_| config);

    CompleteRange {
        payload,
        config,
        receipts,
        account_data,
    }
}

#[cfg(test)]
mod tests {
    use ruma::{api::client::sync::sync_events::v5::response::Room as ResponseRoom, room_id};
    use serde_json::{json, value::to_raw_value};

    use super::*;

    #[test]
    fn extension_only_range_commits_without_a_room_payload() {
        let room_id = room_id!("!extension-only:example.com");
        let range = assemble(None, Vec::new().into_iter(), None, Vec::new(), 7);

        let mut range = publish(room_id, range); // room_updates consumes the configuration.

        assert_eq!(range.room_updates().collect::<Vec<_>>(), [(room_id, None)]);
        assert!(range.into_payloads().is_empty());
    }

    #[test]
    fn payload_range_commits_its_configuration() {
        let room_id = room_id!("!payload:example.com");
        let range = assemble(
            Some(ResponseRoom::default()),
            Vec::new().into_iter(),
            None,
            Vec::new(),
            7,
        );

        let mut range = publish(room_id, range);

        assert_eq!(
            range.room_updates().collect::<Vec<_>>(),
            [(room_id, Some(7))]
        );
        assert_eq!(range.into_payloads().len(), 1);
    }

    #[test]
    fn extension_outputs_are_taken_once_without_removing_the_complete_range() {
        let room_id = room_id!("!extension-output:example.com");
        let receipt = Raw::from_json(
            to_raw_value(&json!({"content": {}})).expect("test receipt should serialize"),
        );

        let account_data = Raw::from_json(
            to_raw_value(&json!({"type": "m.tag", "content": {"tags": {}}}))
                .expect("test account data should serialize"),
        );

        let range = CompleteRange {
            payload: None,
            config: None,
            receipts: Some(receipt),
            account_data: vec![account_data],
        };

        let ranges = [(room_id.to_owned(), range)].into();
        let mut results = Results { ranges };

        assert!(results.take_receipts(room_id).is_some());
        assert!(results.take_receipts(room_id).is_none());

        let count = results
            .take_account_data(room_id)
            .map(|events| events.len());

        assert_eq!(Some(1), count);
        assert!(results.take_account_data(room_id).is_none());
        assert!(results.ranges.contains_key(room_id));
    }

    fn publish(room_id: &RoomId, range: CompleteRange) -> Results {
        Results {
            ranges: [(room_id.to_owned(), range)].into(),
        }
    }
}
