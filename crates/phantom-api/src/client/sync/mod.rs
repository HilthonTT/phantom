mod v3;
mod v5;

use futures::{StreamExt, pin_mut};
use phantom_core::{
    Error, Result,
    matrix::{PduCount, PduEvent},
    stream::{BroadbandExt, ReadyExt},
};
use phantom_service::Services;
use ruma::{RoomId, UserId};

pub(crate) use self::{v3::sync_events_route, v5::sync_events_v5_route};

/// Loads up to `limit` events after `roomsincecount`, oldest first.
///
/// Returns the events, whether older events after `roomsincecount` were left
/// out, and the count of the newest event at or before `next_batch`.
/// An unreadable event is skipped unless it is the newest one.
async fn load_timeline(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    roomsincecount: PduCount,
    next_batch: Option<PduCount>,
    limit: usize,
) -> Result<(Vec<(PduCount, PduEvent)>, bool, PduCount), Error> {
    let until = next_batch.map(|count| count.saturating_add(1));
    let pdus = services
        .rooms
        .timeline
        .pdus_rev(Some(sender_user), room_id, until);

    // Take the last events for the timeline.
    pin_mut!(pdus);
    let mut timeline_pdus = Vec::new();
    let mut last_timeline_count = PduCount::max();
    let mut first = true;
    let mut limited = false;

    while let Some(pdu) = pdus.next().await {
        let (pducount, pdu) = match pdu {
            Ok(pdu) => pdu,
            Err(error) if first => return Err(error),
            Err(_) => continue,
        };

        if first {
            first = false;
            last_timeline_count = matches!(pducount, PduCount::Normal(_))
                .then_some(pducount)
                .unwrap_or_else(PduCount::max);
        }

        if pducount <= roomsincecount {
            break;
        }

        if timeline_pdus.len() == limit {
            limited = true;
            break;
        }

        timeline_pdus.push((pducount, pdu));
    }

    timeline_pdus.reverse();

    Ok((timeline_pdus, limited, last_timeline_count))
}

/// Whether `sender_user` and `user_id` share an encrypted room other than
/// `ignore_room`.
async fn share_encrypted_room(
    services: &Services,
    sender_user: &UserId,
    user_id: &UserId,
    ignore_room: Option<&RoomId>,
) -> bool {
    services
        .rooms
        .state_cache
        .get_shared_rooms(sender_user, user_id)
        .ready_filter(|&room_id| Some(room_id) != ignore_room)
        .map(ToOwned::to_owned)
        .broad_any(async |other_room_id| {
            services
                .rooms
                .state_accessor
                .is_encrypted_room(&other_room_id)
                .await
        })
        .await
}
