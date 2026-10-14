//! The notified-event list behind `GET /notifications`.
//!
//! One row per event that notified a user, keyed by `(user_id, count)` where
//! `count` is the event's timeline position. Counts come from the server-wide
//! counter, so they are unique across rooms and increase with time: a user's
//! rows read newest first by walking their prefix backwards, and a page
//! token is simply the count of the last row returned.
//!
//! The count is a `u64` because the key decoder cannot read narrower
//! integers. The value holds the room's short id rather than the room and
//! event ids, since the short id and the count together are the event's
//! `PduId`.

use std::sync::Arc;

use futures::StreamExt;
use phantom_core::{
    Result,
    stream::{ReadyExt, TryIgnore},
};
use phantom_database::{Json, Map, deserialize};
use ruma::{UserId, push::Action};
use serde::{Deserialize, Serialize};

use crate::rooms::short::ShortRoomId;

/// What is stored for one notified event.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Notified {
    /// When the event was notified, in milliseconds since the Unix epoch.
    pub ts: u64,

    /// The room the event is in, as its short id.
    pub sroomid: ShortRoomId,

    /// The profile tag of the rule that matched. Push rules no longer carry
    /// profile tags, so this is always empty today; it is stored so the
    /// response can carry one should they return.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,

    /// The actions the user's push rules gave the event.
    pub actions: Vec<Action>,
}

impl Notified {
    #[must_use]
    pub fn is_highlight(&self) -> bool {
        self.actions.iter().any(Action::is_highlight)
    }
}

/// Just the timestamp of a row, for the sweep.
#[derive(Deserialize)]
struct Stamp {
    ts: u64,
}

pub(super) fn put(map: &Arc<Map>, user_id: &UserId, count: u64, notified: &Notified) -> Result {
    map.put((user_id, count), Json(notified))
}

/// Up to `limit` of the user's rows below `from` (or from the newest when
/// `None`), newest first, and the token for the next page.
///
/// One row beyond the page is read to learn whether another page exists, so
/// the last page returns no token and a client stops without asking for an
/// empty one.
pub(super) async fn page(
    map: &Arc<Map>,
    user_id: &UserId,
    from: Option<u64>,
    limit: usize,
    only_highlight: bool,
) -> (Vec<(u64, Notified)>, Option<u64>) {
    // `from` is exclusive; a token of 0 has nothing below it.
    let start = match from {
        Some(0) => return (Vec::new(), None),
        Some(from) => from.saturating_sub(1),
        None => u64::MAX,
    };

    let mut rows: Vec<(u64, Notified)> = map
        .rev_stream_from(&(user_id, start))
        .ignore_err()
        .ready_take_while(|((user, _), _): &((&str, u64), Notified)| *user == user_id.as_str())
        .ready_filter(|(_, notified): &((&str, u64), Notified)| {
            !only_highlight || notified.is_highlight()
        })
        .map(|((_, count), notified)| (count, notified))
        .boxed()
        .take(limit.saturating_add(1))
        .collect()
        .await;

    let next = (rows.len() > limit).then(|| {
        rows.truncate(limit);
        rows.last().map(|(count, _)| *count)
    });

    (rows, next.flatten())
}

/// Removes rows older than `max_age_ms` (0 keeps rows of any age) and each
/// user's rows beyond their newest `max_per_user` (0 keeps any number).
/// Returns how many rows went.
///
/// One backward pass over the table visits each user's rows newest first, so
/// counting how many of the user's rows have been seen is enough to know
/// which are over the cap.
pub(super) async fn prune(
    map: &Arc<Map>,
    now_ms: u64,
    max_age_ms: u64,
    max_per_user: usize,
) -> usize {
    if max_age_ms == 0 && max_per_user == 0 {
        return 0;
    }

    let cutoff = now_ms.saturating_sub(max_age_ms);
    let mut current: Option<Vec<u8>> = None;
    let mut seen = 0_usize;
    let mut removed = 0_usize;

    map.rev_raw_stream()
        .ignore_err()
        .boxed()
        .ready_for_each(|(key, val)| {
            let Ok((user, _)) = deserialize::<(&str, u64)>(key) else {
                return;
            };

            if current.as_deref() != Some(user.as_bytes()) {
                current = Some(user.as_bytes().to_vec());
                seen = 0;
            }

            seen = seen.saturating_add(1);

            let over_cap = max_per_user != 0 && seen > max_per_user;
            let too_old = max_age_ms != 0
                && serde_json::from_slice::<Stamp>(val).is_ok_and(|stamp| stamp.ts < cutoff);

            if (over_cap || too_old) && map.remove(key).is_ok() {
                removed = removed.saturating_add(1);
            }
        })
        .await;

    removed
}
