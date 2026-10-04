mod read_markers;
mod receipt;

use futures::future::try_join;
use phantom_core::{Err, Result, debug, err, matrix::PduCount, matrix::PduId};
use phantom_service::Services;
use ruma::{EventId, RoomId, UserId};

pub(crate) use self::{read_markers::set_read_marker_route, receipt::create_receipt_route};

/// Resolves `event` to its timeline position and stores the private read
/// marker there.
///
/// Returns whether the marker advanced. A backfilled event carries no forward
/// position, so it is skipped like a non-advancing write rather than failing
/// the request. Phantom keeps one private marker per room, so the receipt's
/// thread is not recorded.
async fn set_private_marker(
    services: &Services,
    room_id: &RoomId,
    user_id: &UserId,
    event: &EventId,
) -> Result<bool> {
    let (pdu_id, shortroomid) = try_join(
        services.rooms.timeline.get_pdu_id(event),
        services.rooms.short.get_shortroomid(room_id),
    )
    .await
    .map_err(|_| err!(Request(NotFound("Event not found."))))?;

    let pdu_id = PduId::from(pdu_id);

    if pdu_id.shortroomid != shortroomid {
        return Err!(Request(NotFound("Event not found.")));
    }

    let PduCount::Normal(count) = pdu_id.shorteventid else {
        debug!(%user_id, %room_id, %event, "Skipping private read marker at a backfilled event");
        return Ok(false);
    };

    let advances = services
        .rooms
        .read_receipt
        .private_read_get_count(room_id, user_id)
        .await
        .map_or(true, |current| count > current);

    if advances {
        services
            .rooms
            .read_receipt
            .private_read_set(room_id, user_id, count)?;
    }

    Ok(advances)
}

/// Clears the room's notification counts after a receipt advanced.
///
/// Phantom keeps counts per room only, so per-thread counts and the push
/// gateway badge are not touched.
fn reset_notification_counts(services: &Services, user_id: &UserId, room_id: &RoomId) {
    services
        .rooms
        .user
        .reset_notification_counts(user_id, room_id);
}
