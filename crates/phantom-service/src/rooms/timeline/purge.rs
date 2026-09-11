//! Removing a room's events, when the room itself is going.
//!
//! A pdu id is the room's short id followed by the event's position in it, so
//! a room's whole timeline is one prefix of `pduid_pdu` and the walk is
//! ordered. The two columns keyed by event id instead — the id-to-position
//! map, and the outliers — cannot be reached that way, so each event's id is
//! read out of the stored PDU as the walk passes it.
//!
//! What is deliberately not swept here is the outlier column at large. An
//! outlier is an event the server was handed and could not place in any room,
//! so there is no room to purge it with; it is the backfill and
//! state-resolution paths that resolve or abandon those. The removal below
//! only covers the case of an event that was an outlier before it was
//! accepted, whose entry the append path is expected to have cleared already.

use futures::TryStreamExt;
use phantom_core::{Result, implement, matrix::pdu::RawPduId, trace};
use ruma::{OwnedEventId, RoomId};
use serde::Deserialize;

use crate::rooms::short::ShortRoomId;

/// Just the event id out of a stored PDU.
///
/// The whole event is on disk in front of us and all that is wanted is the
/// one field the other columns are keyed by.
#[derive(Deserialize)]
struct ExtractEventId {
    event_id: OwnedEventId,
}

/// Deletes every PDU of a room, returning how many there were.
///
/// Each event's relations and soft-fail mark go with it, as does the
/// unredacted original retained for it, since neither is keyed by room and so
/// neither can be swept once the timeline they belonged to is gone.
///
/// The room's search tokens are *not* deindexed one message at a time: they
/// are keyed by short room id, so the caller drops them wholesale. That also
/// catches the tokens of messages that have since been redacted, which a walk
/// of the PDUs would no longer be able to reconstruct.
#[implement(super::Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub(in crate::rooms) async fn delete_all_pdus(
    &self,
    room_id: &RoomId,
    shortroomid: ShortRoomId,
) -> Result<usize> {
    let prefix = shortroomid.to_be_bytes();
    let _cork = self.db.engine.cork_guard();

    self.db
        .pduid_pdu
        .raw_stream_prefix(&prefix)
        .try_fold(0_usize, async |deleted, (key, value)| {
            let pdu_id = RawPduId::from(key);
            let ExtractEventId { event_id } = serde_json::from_slice(value)?;

            self.db.pduid_pdu.remove(key).ok();
            self.db.eventid_pduid.remove(event_id.as_bytes()).ok();
            self.db.eventid_outlierpdu.remove(event_id.as_bytes()).ok();

            self.services
                .pdu_metadata
                .purge_event(&pdu_id.shorteventid(), &event_id)
                .await;

            self.services.retention.purge_original(&event_id);

            trace!(%event_id, %room_id, "Purged event");

            Ok(deleted.saturating_add(1))
        })
        .await
}
