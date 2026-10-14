use axum::extract::State;
use phantom_core::{
    Result, err,
    matrix::pdu::{PduCount, PduId},
};
use ruma::{MilliSecondsSinceUnixEpoch, UInt, api::client::push::get_notifications};

use crate::router::Ruma;

/// Page size when the client names none.
const DEFAULT_LIMIT: usize = 50;

/// The largest page served, whatever the client asks for.
const MAX_LIMIT: usize = 100;

/// # `GET /_matrix/client/r0/notifications/`
///
/// Paginate through the list of events the user has been, or would have been
/// notified about.
///
/// The list is what the append path recorded for the user, newest first and
/// bounded by `notification_retention_days` and `notification_max_per_user`.
/// An event is reported as read once the user's counts in its room were
/// reset after it — by a receipt or by the user sending there — which keeps
/// the flag in step with the unread counts sync serves. Events since deleted
/// or redacted are left out, so a page can be shorter than `limit` while
/// another follows.
pub(crate) async fn get_notifications_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_notifications::v3::Request>,
) -> Result<get_notifications::v3::Response> {
    use get_notifications::v3::{Notification, Response};

    let sender_user = body.sender_user();

    let from = body
        .body
        .from
        .as_deref()
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|e| err!(Request(InvalidParam("Invalid `from' parameter: {e}"))))?;

    let limit = body
        .body
        .limit
        .map_or(DEFAULT_LIMIT, |limit| {
            usize::try_from(u64::from(limit)).unwrap_or(MAX_LIMIT)
        })
        .clamp(1, MAX_LIMIT);

    let only_highlight = body
        .body
        .only
        .as_deref()
        .is_some_and(|only| only.contains("highlight"));

    let (rows, next) = services
        .pusher
        .notifications_page(sender_user, from, limit, only_highlight)
        .await;

    let mut notifications = Vec::with_capacity(rows.len());
    for (count, notified) in rows {
        let pdu_id = PduId {
            shortroomid: notified.sroomid,
            shorteventid: PduCount::Normal(count),
        };

        let Ok(pdu) = services
            .rooms
            .timeline
            .get_pdu_from_id(&pdu_id.into())
            .await
        else {
            continue;
        };

        if pdu.is_redacted() {
            continue;
        }

        let read = services
            .rooms
            .user
            .last_notification_read(sender_user, &pdu.room_id)
            .await
            > count;

        let ts = UInt::try_from(notified.ts).map_or_else(
            |_| MilliSecondsSinceUnixEpoch::now(),
            MilliSecondsSinceUnixEpoch,
        );

        let room_id = pdu.room_id.clone();
        let mut notification = Notification::new(
            notified.actions,
            pdu.into_sync_room_event(),
            read,
            room_id,
            ts,
        );
        notification.profile_tag = notified.tag;

        notifications.push(notification);
    }

    let mut response = Response::new(notifications);
    response.next_token = next.map(|count| count.to_string());

    Ok(response)
}
