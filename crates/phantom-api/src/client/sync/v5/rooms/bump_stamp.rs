use futures::{TryStreamExt, pin_mut};
use phantom_core::{
    Result,
    matrix::{Event, PduCount, PduEvent},
    stream::TryReadyExt,
};
use phantom_service::Services;
use ruma::{RoomId, UInt, UserId, events::TimelineEventType};

/// Event types that move a room up its lists.
///
/// Matched by name: ruma only types the poll and beacon events behind
/// unstable features phantom leaves off.
static DEFAULT_BUMP_TYPES: [&str; 6] = [
    "m.call.invite",
    "m.poll.start",
    "m.room.encrypted",
    "m.room.message",
    "m.sticker",
    "org.matrix.msc3672.beacon",
];

pub(super) async fn room_bump_stamp(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    roomsince: PduCount,
    next_batch: PduCount,
    last_timeline_count: PduCount,
) -> Result<Option<UInt>> {
    if last_timeline_count <= roomsince {
        return Ok(None);
    }

    let bumpable_pdus = services
        .rooms
        .timeline
        .pdus_rev(
            Some(sender_user),
            room_id,
            Some(next_batch.saturating_add(1)),
        )
        .ready_try_take_while(move |&(pdu_count, _)| Ok(pdu_count > roomsince))
        .ready_try_filter_map(|(pdu_count, pdu)| {
            Ok(is_bumpable_pdu(&pdu, sender_user)
                .then(|| pdu_count.into_signed().try_into().ok())
                .flatten())
        });

    pin_mut!(bumpable_pdus);
    bumpable_pdus.try_next().await
}

fn is_bumpable_pdu(pdu: &PduEvent, sender_user: &UserId) -> bool {
    if pdu.is_redacted() {
        return false;
    }

    if *pdu.event_type() == TimelineEventType::RoomMember {
        return pdu.state_key() == Some(sender_user.as_str());
    }

    let event_type = pdu.event_type().to_string();

    DEFAULT_BUMP_TYPES.contains(&event_type.as_str())
}

#[cfg(test)]
mod tests {
    use phantom_core::matrix::PduEvent;
    use ruma::{events::TimelineEventType, user_id};
    use serde_json::json;

    use super::{DEFAULT_BUMP_TYPES, is_bumpable_pdu};

    fn pdu(kind: &TimelineEventType, state_key: Option<&str>, redacted: bool) -> PduEvent {
        let mut pdu = json!({
            "event_id": "$event:example.com",
            "room_id": "!room:example.com",
            "sender": "@alice:example.com",
            "origin_server_ts": 1,
            "type": kind.to_string(),
            "content": {},
            "prev_events": [],
            "depth": 1,
            "auth_events": [],
            "hashes": { "sha256": "" },
        });

        if let Some(state_key) = state_key {
            pdu["state_key"] = json!(state_key);
        }

        if redacted {
            pdu["unsigned"] = json!({ "redacted_because": {} });
        }

        serde_json::from_value(pdu).expect("valid test pdu")
    }

    #[test]
    fn default_bump_types_bump() {
        let sender = user_id!("@alice:example.com");

        for kind in DEFAULT_BUMP_TYPES {
            let kind = TimelineEventType::from(kind);
            assert!(is_bumpable_pdu(&pdu(&kind, None, false), sender));
        }
    }

    #[test]
    fn non_bump_type_does_not_bump() {
        let sender = user_id!("@alice:example.com");
        let pdu = pdu(&TimelineEventType::RoomName, Some(""), false);

        assert!(!is_bumpable_pdu(&pdu, sender));
    }

    #[test]
    fn own_membership_bumps() {
        let sender = user_id!("@alice:example.com");
        let pdu = pdu(&TimelineEventType::RoomMember, Some(sender.as_str()), false);

        assert!(is_bumpable_pdu(&pdu, sender));
    }

    #[test]
    fn other_membership_does_not_bump() {
        let sender = user_id!("@alice:example.com");
        let pdu = pdu(
            &TimelineEventType::RoomMember,
            Some("@bob:example.com"),
            false,
        );

        assert!(!is_bumpable_pdu(&pdu, sender));
    }

    #[test]
    fn redacted_pdu_does_not_bump() {
        let sender = user_id!("@alice:example.com");
        let pdu = pdu(&TimelineEventType::RoomMessage, None, true);

        assert!(!is_bumpable_pdu(&pdu, sender));
    }
}
