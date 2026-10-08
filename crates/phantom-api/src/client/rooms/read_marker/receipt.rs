use std::collections::BTreeMap;

use axum::extract::State;
use phantom_core::{Err, Result};
use phantom_service::Services;
use ruma::{
    EventId, MilliSecondsSinceUnixEpoch, OwnedEventId,
    api::client::receipt::create_receipt::{self, v3::ReceiptType as CreateReceiptType},
    events::{
        RoomAccountDataEventType,
        fully_read::{FullyReadEvent, FullyReadEventContent},
        receipt::{Receipt, ReceiptEvent, ReceiptEventContent, ReceiptThread, ReceiptType},
        relation::RelationType,
    },
};
use serde::Deserialize;

use super::{reset_notification_counts, set_private_marker};
use crate::{client::utils::ping_presence, router::Ruma};

/// # `POST /_matrix/client/r0/rooms/{roomId}/receipt/{receiptType}/{eventId}`
///
/// Sets private read marker and public read receipt EDU.
pub(crate) async fn create_receipt_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_receipt::v3::Request>,
) -> Result<create_receipt::v3::Response> {
    let sender_user = body.sender_user();

    // MSC3771: thread_id MUST NOT be provided with `m.fully_read`.
    if matches!(&body.receipt_type, CreateReceiptType::FullyRead)
        && !matches!(body.thread, ReceiptThread::Unthreaded)
    {
        return Err!(Request(InvalidParam(
            "thread_id must not be set for m.fully_read receipts"
        )));
    }

    // MSC3771: a present thread_id must be a non-empty string.
    if body.thread.as_str() == Some("") {
        return Err!(Request(InvalidParam(
            "thread_id must be a non-empty string"
        )));
    }

    // MSC3771: thread_id is either `"main"` or a thread root event id (which
    // starts with `$`).
    if !matches!(
        &body.thread,
        ReceiptThread::Unthreaded | ReceiptThread::Main | ReceiptThread::Thread(_)
    ) {
        return Err!(Request(InvalidParam(
            "thread_id must be either \"main\" or a thread root event id"
        )));
    }

    // MSC3771: event_id must belong to the thread the receipt targets.
    if matches!(&body.thread, ReceiptThread::Main | ReceiptThread::Thread(_)) {
        let resolved = thread_root_of(&services, &body.event_id).await;

        let in_thread = match (&body.thread, resolved.as_deref()) {
            (ReceiptThread::Main, None) => true,
            (ReceiptThread::Thread(root), Some(parent)) => &**root == parent,
            (ReceiptThread::Thread(root), None) => **root == *body.event_id,
            _ => false,
        };

        if !in_thread {
            return Err!(Request(InvalidParam(
                "event_id is not related to the given thread_id"
            )));
        }
    }

    let advanced = match body.receipt_type {
        CreateReceiptType::FullyRead => {
            let fully_read_event =
                FullyReadEvent::new(FullyReadEventContent::new(body.event_id.clone()));
            services
                .account_data
                .update(
                    Some(&body.room_id),
                    sender_user,
                    RoomAccountDataEventType::FullyRead,
                    &serde_json::to_value(fully_read_event)?,
                )
                .await?;

            false
        }
        CreateReceiptType::Read => {
            let receipt_content = BTreeMap::from_iter([(
                body.event_id.clone(),
                BTreeMap::from_iter([(
                    ReceiptType::Read,
                    BTreeMap::from_iter([(sender_user.to_owned(), {
                        let mut receipt = Receipt::new(MilliSecondsSinceUnixEpoch::now());
                        receipt.thread = body.thread.clone();
                        receipt
                    })]),
                )]),
            )]);

            services
                .rooms
                .read_receipt
                .readreceipt_update(
                    sender_user,
                    &body.room_id,
                    &ReceiptEvent::new(body.room_id.clone(), ReceiptEventContent(receipt_content)),
                )
                .await?;

            ping_presence(&services, &body, sender_user).await.ok();

            true
        }
        CreateReceiptType::ReadPrivate => {
            set_private_marker(&services, &body.room_id, sender_user, &body.event_id).await?
        }
        _ => {
            return Err!(Request(InvalidParam(warn!(
                "Received unknown read receipt type: {}",
                &body.receipt_type
            ))));
        }
    };

    if advanced {
        reset_notification_counts(&services, sender_user, &body.room_id);
    }

    Ok(create_receipt::v3::Response::new())
}

#[derive(Deserialize)]
struct ExtractThreadRoot {
    #[serde(rename = "m.relates_to")]
    relates_to: ThreadRelation,
}

#[derive(Deserialize)]
struct ThreadRelation {
    rel_type: RelationType,
    event_id: OwnedEventId,
}

/// The root of the thread `event_id` belongs to, from its `m.thread`
/// relation; `None` for an event outside any thread.
async fn thread_root_of(services: &Services, event_id: &EventId) -> Option<OwnedEventId> {
    let pdu = services.rooms.timeline.get_pdu(event_id).await.ok()?;

    pdu.get_content::<ExtractThreadRoot>()
        .ok()
        .map(|content| content.relates_to)
        .filter(|relation| relation.rel_type == RelationType::Thread)
        .map(|relation| relation.event_id)
}
