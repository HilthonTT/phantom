use futures::TryStreamExt;
use phantom_core::{Result, implement, matrix::pdu::RawPduId, trace};
use ruma::{OwnedEventId, RoomId};
use serde::Deserialize;

use crate::rooms::short::ShortRoomId;

#[derive(Deserialize)]
struct ExtractEventId {
    event_id: OwnedEventId,
}

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
