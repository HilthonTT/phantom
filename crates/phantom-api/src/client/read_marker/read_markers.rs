use std::collections::BTreeMap;

use axum::extract::State;
use phantom_core::Result;
use ruma::{
    MilliSecondsSinceUnixEpoch,
    api::client::read_marker::set_read_marker,
    events::{
        RoomAccountDataEventType,
        fully_read::{FullyReadEvent, FullyReadEventContent},
        receipt::{Receipt, ReceiptEvent, ReceiptEventContent, ReceiptType},
    },
};

use super::{reset_notification_counts, set_private_marker};
use crate::{client::utils::ping_presence, router::Ruma};

/// # `POST /_matrix/client/r0/rooms/{roomId}/read_markers`
///
/// Sets different types of read markers.
///
/// - Updates fully-read account data event to `fully_read`
/// - If `read_receipt` is set: Update private marker and public read receipt
///   EDU
pub(crate) async fn set_read_marker_route(
    State(services): State<crate::router::State>,
    body: Ruma<set_read_marker::v3::Request>,
) -> Result<set_read_marker::v3::Response> {
    let sender_user = body.sender_user();

    if let Some(event) = &body.fully_read {
        let fully_read_event = FullyReadEvent::new(FullyReadEventContent::new(event.clone()));

        services
            .account_data
            .update(
                Some(&body.room_id),
                sender_user,
                RoomAccountDataEventType::FullyRead,
                &serde_json::to_value(fully_read_event)?,
            )
            .await
            .ok();
    }

    let private_advanced = match &body.private_read_receipt {
        None => false,
        Some(event) => set_private_marker(&services, &body.room_id, sender_user, event).await?,
    };

    let public_advanced = match &body.read_receipt {
        None => false,
        Some(event) => {
            let receipt_content = BTreeMap::from_iter([(
                event.to_owned(),
                BTreeMap::from_iter([(
                    ReceiptType::Read,
                    BTreeMap::from_iter([(
                        sender_user.to_owned(),
                        Receipt::new(MilliSecondsSinceUnixEpoch::now()),
                    )]),
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
    };

    if private_advanced || public_advanced {
        reset_notification_counts(&services, sender_user, &body.room_id);
    }

    Ok(set_read_marker::v3::Response::new())
}
