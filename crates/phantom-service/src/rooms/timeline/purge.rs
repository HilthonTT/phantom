use std::collections::HashSet;

use futures::{StreamExt, TryStreamExt};
use phantom_core::{
    Result, implement, info,
    matrix::pdu::{PduCount, RawPduId},
    trace,
};
use ruma::{OwnedEventId, OwnedUserId, RoomId, UInt};
use serde::{Deserialize, Serialize};

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

/// Where a history purge stops: events before this are purged.
#[derive(Clone, Copy, Debug)]
pub enum HistoryCutoff {
    /// Events sent before this time, in milliseconds since the epoch, by the
    /// sender's own clock.
    Timestamp(u64),

    /// Events placed in the timeline before this one.
    Event(PduCount),
}

/// What a history purge removed and kept.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PurgedHistory {
    pub purged: usize,

    /// Events before the cutoff kept because the room's state needs them.
    pub kept_state: usize,

    /// Events before the cutoff kept because a local user sent them and local
    /// events were not asked for.
    pub kept_local: usize,
}

#[derive(Deserialize)]
struct ExtractPurge {
    event_id: OwnedEventId,
    sender: OwnedUserId,
    origin_server_ts: UInt,
    state_key: Option<serde::de::IgnoredAny>,
    content: ExtractBody,
}

#[derive(Default, Deserialize)]
struct ExtractBody {
    #[serde(default)]
    body: Option<serde_json::Value>,
}

/// Deletes a room's timeline events before the cutoff, keeping what the room
/// cannot do without: every state event, since state resolution and the auth
/// chains of later events are built from them, and the room's forward
/// extremities, which the next event sent will name as its parents.
///
/// Events local users sent are kept unless `include_local` is set: other
/// servers in the room hold their own copies of remote events, but a local
/// event exists nowhere else once purged here. A purged remote event can be
/// fetched back by backfill if a client pages that far.
#[implement(super::Service)]
#[tracing::instrument(skip(self), level = "info")]
pub async fn purge_history(
    &self,
    room_id: &RoomId,
    cutoff: HistoryCutoff,
    include_local: bool,
) -> Result<PurgedHistory> {
    let shortroomid = self.services.short.get_shortroomid(room_id).await?;

    let extremities: HashSet<OwnedEventId> = self
        .services
        .state
        .get_forward_extremities(room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let prefix = shortroomid.to_be_bytes();
    let _cork = self.db.engine.cork_guard();

    let purged = self
        .db
        .pduid_pdu
        .raw_stream_prefix(&prefix)
        .try_fold(
            PurgedHistory::default(),
            async |mut purged, (key, value)| {
                let pdu_id = RawPduId::from(key);
                let Ok(pdu) = serde_json::from_slice::<ExtractPurge>(value) else {
                    return Ok(purged);
                };

                let before = match cutoff {
                    HistoryCutoff::Timestamp(ts) => u64::from(pdu.origin_server_ts) < ts,
                    HistoryCutoff::Event(count) => pdu_id.pdu_count() < count,
                };

                if !before || extremities.contains(&pdu.event_id) {
                    return Ok(purged);
                }

                if pdu.state_key.is_some() {
                    purged.kept_state = purged.kept_state.saturating_add(1);
                    return Ok(purged);
                }

                if !include_local
                    && self
                        .services
                        .server_state
                        .server_is_ours(pdu.sender.server_name())
                {
                    purged.kept_local = purged.kept_local.saturating_add(1);
                    return Ok(purged);
                }

                if let Some(serde_json::Value::String(body)) = &pdu.content.body {
                    self.services
                        .search
                        .deindex_pdu(shortroomid, &pdu_id, body)
                        .ok();
                }

                let event_id = pdu.event_id;
                self.db.pduid_pdu.remove(key).ok();
                self.db.eventid_pduid.remove(event_id.as_bytes()).ok();
                self.db.eventid_outlierpdu.remove(event_id.as_bytes()).ok();
                self.db.eventid_purged.insert(event_id.as_bytes(), []).ok();

                self.services
                    .pdu_metadata
                    .purge_event(&pdu_id.shorteventid(), &event_id)
                    .await;

                self.services.retention.purge_original(&event_id);

                trace!(%event_id, %room_id, "Purged event from history");

                purged.purged = purged.purged.saturating_add(1);

                Ok(purged)
            },
        )
        .await?;

    info!(%room_id, ?cutoff, ?purged, "Purged room history");

    Ok(purged)
}

#[cfg(test)]
mod tests {
    use super::ExtractPurge;

    #[test]
    fn a_state_event_is_told_from_a_message() {
        let state: ExtractPurge = serde_json::from_str(
            r#"{"event_id":"$a","sender":"@u:x","origin_server_ts":1,"type":"m.room.name",
                "state_key":"","content":{"name":"n"}}"#,
        )
        .expect("parses");
        let message: ExtractPurge = serde_json::from_str(
            r#"{"event_id":"$b","sender":"@u:x","origin_server_ts":2,"type":"m.room.message",
                "content":{"body":"hi","msgtype":"m.text"}}"#,
        )
        .expect("parses");

        assert!(state.state_key.is_some());
        assert!(message.state_key.is_none());
        assert_eq!(
            message.content.body,
            Some(serde_json::Value::String("hi".into()))
        );
    }

    /// An empty state key is still a state event; it must not read as absent.
    #[test]
    fn an_empty_state_key_counts() {
        let pdu: ExtractPurge = serde_json::from_str(
            r#"{"event_id":"$a","sender":"@u:x","origin_server_ts":1,"state_key":"",
                "content":{}}"#,
        )
        .expect("parses");

        assert!(pdu.state_key.is_some());
    }
}
