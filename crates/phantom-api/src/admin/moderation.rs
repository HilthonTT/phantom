//! The admin API's moderation actions: the runtime server block list, media
//! quarantine, and purging old room history and media.

use std::collections::BTreeSet;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::IntoResponse,
};
use futures::StreamExt;
use phantom_core::{Err, Result, err, time::now_millis, time::now_secs};
use phantom_service::{
    media::Origin,
    ops::moderation::{Restriction, exact_pattern},
    rooms::timeline::HistoryCutoff,
};
use ruma::{
    MxcUri, OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedServerName, OwnedUserId,
    events::TimelineEventType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::router::{AdminAuth, Routes, State as RouterState, get, post, put};

pub(super) fn register(router: Routes) -> Routes {
    router
        .route(
            "/_phantom/admin/v1/federation/blocks",
            get(blocks).post(block).delete(unblock),
        )
        .route("/_phantom/admin/v1/media/quarantine", get(quarantined))
        .route(
            "/_phantom/admin/v1/media/{server_name}/{media_id}/quarantine",
            put(quarantine).delete(unquarantine),
        )
        .route(
            "/_phantom/admin/v1/rooms/{room_id}/media/quarantine",
            post(quarantine_room_media),
        )
        .route(
            "/_phantom/admin/v1/users/{user_id}/media/quarantine",
            post(quarantine_user_media),
        )
        .route(
            "/_phantom/admin/v1/rooms/{room_id}/purge_history",
            post(purge_history),
        )
        .route("/_phantom/admin/v1/media/purge", post(purge_media))
}

#[derive(Serialize)]
pub(super) struct BlockRow {
    pattern: String,

    /// "runtime" for the block list kept in the database, which the API
    /// edits; "config" for `forbidden_remote_server_names`, which only the
    /// config file changes.
    source: &'static str,
    reason: Option<String>,
    by: Option<String>,
    blocked_at_ms: Option<u64>,
}

/// # `GET /_phantom/admin/v1/federation/blocks`
///
/// Every pattern refusing remote servers: the config file's first, then the
/// runtime list, oldest first. Both are regular expressions matched against
/// a server's host.
pub(super) async fn blocks(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let config = services
        .moderation
        .config_blocks()
        .into_iter()
        .map(|pattern| BlockRow {
            pattern,
            source: "config",
            reason: None,
            by: None,
            blocked_at_ms: None,
        });

    let runtime = services
        .moderation
        .blocks()
        .into_iter()
        .map(|block| BlockRow {
            pattern: block.pattern,
            source: "runtime",
            reason: block.reason,
            by: block.by.map(|user| user.to_string()),
            blocked_at_ms: Some(block.at_ms),
        });

    Ok(Json(config.chain(runtime).collect::<Vec<_>>()))
}

/// A block or unblock names exactly one of a server or a pattern.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct BlockTarget {
    server: Option<OwnedServerName>,
    pattern: Option<String>,
    reason: Option<String>,
}

/// # `POST /_phantom/admin/v1/federation/blocks`
///
/// Adds to the runtime block list, by `server` (that host exactly) or by
/// `pattern` (a regular expression, as the config file's list takes), with an
/// optional `reason`. A blocked server's requests are refused, nothing is
/// sent to it, joins do not go through it and its media is not fetched.
pub(super) async fn block(
    State(services): State<RouterState>,
    admin: AdminAuth,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: BlockTarget = super::body(&body)?;
    let pattern = pattern_of(&body)?;

    let added = services
        .moderation
        .block(&pattern, body.reason, &admin.user_id)
        .await?;

    Ok(Json(json!({ "pattern": pattern, "added": added })))
}

/// # `DELETE /_phantom/admin/v1/federation/blocks`
///
/// Removes from the runtime block list: by `pattern`, that pattern as it was
/// written; by `server`, every runtime pattern matching it. Answers with the
/// patterns removed, and with the config file's patterns that still refuse
/// the server, which only the config file can lift.
pub(super) async fn unblock(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: BlockTarget = super::body(&body)?;

    match (&body.server, &body.pattern) {
        (Some(server), None) => {
            let removed = services.moderation.unblock_server(server).await?;
            let still_blocked_by = services
                .moderation
                .why_forbidden(server, Restriction::Federation);

            if removed.is_empty() && still_blocked_by.is_empty() {
                return Err!(Request(NotFound("{server} is not blocked.")));
            }

            Ok(Json(json!({
                "removed": removed,
                "still_blocked_by": still_blocked_by,
            })))
        }
        (None, Some(pattern)) => {
            services.moderation.unblock_pattern(pattern).await?;

            Ok(Json(
                json!({ "removed": [pattern], "still_blocked_by": [] }),
            ))
        }
        _ => Err!(Request(InvalidParam("Give either a server or a pattern."))),
    }
}

fn pattern_of(body: &BlockTarget) -> Result<String> {
    match (&body.server, &body.pattern) {
        (Some(server), None) => Ok(exact_pattern(server)),
        (None, Some(pattern)) if !pattern.is_empty() => Ok(pattern.clone()),
        _ => Err!(Request(InvalidParam("Give either a server or a pattern."))),
    }
}

#[derive(Serialize)]
pub(super) struct QuarantineRow {
    mxc: OwnedMxcUri,
    reason: Option<String>,
    by: Option<String>,
    quarantined_at_ms: u64,

    /// Whether the file is stored here; a quarantine can be set on media
    /// never fetched, to keep it from ever being.
    stored: bool,
}

/// # `GET /_phantom/admin/v1/media/quarantine`
///
/// Every quarantined file, oldest quarantine first.
pub(super) async fn quarantined(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let stored: BTreeSet<OwnedMxcUri> = services
        .media
        .list()
        .await
        .into_iter()
        .map(|media| media.mxc)
        .collect();

    let rows: Vec<QuarantineRow> = services
        .media
        .quarantined()
        .await
        .into_iter()
        .map(|record| QuarantineRow {
            stored: stored.contains(&record.mxc),
            reason: record.reason,
            by: record.by.map(|user| user.to_string()),
            quarantined_at_ms: record.at_ms,
            mxc: record.mxc,
        })
        .collect();

    Ok(Json(rows))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Reason {
    reason: Option<String>,
}

/// # `PUT /_phantom/admin/v1/media/{server_name}/{media_id}/quarantine`
///
/// Quarantines a file: it and its thumbnails are answered as missing and it
/// is never fetched again. The file is kept, so the quarantine can be
/// lifted. Takes an optional `reason`.
pub(super) async fn quarantine(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path((server, media_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Reason = super::body(&body)?;
    let mxc = mxc_of(&server, &media_id)?;

    let added = services
        .media
        .quarantine(&mxc, &admin.user_id, body.reason.as_deref())
        .await?;

    Ok(Json(json!({ "quarantined": usize::from(added) })))
}

/// # `DELETE /_phantom/admin/v1/media/{server_name}/{media_id}/quarantine`
///
/// Lifts a file's quarantine.
pub(super) async fn unquarantine(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path((server, media_id)): Path<(String, String)>,
) -> Result<impl IntoResponse> {
    let mxc = mxc_of(&server, &media_id)?;
    services.media.unquarantine(&mxc).await?;

    Ok(Json(json!({})))
}

/// # `POST /_phantom/admin/v1/rooms/{room_id}/media/quarantine`
///
/// Quarantines every file the room's events refer to, from any server, as a
/// tracked task; answers with the task's ID. Member events are passed over,
/// since an avatar there belongs to the member's profile, not the room.
pub(super) async fn quarantine_room_media(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Reason = super::body(&body)?;

    if !services.rooms.metadata.exists(&room_id).await {
        return Err!(Request(NotFound("The server does not know {room_id}.")));
    }

    let task_services = services.clone();
    let task_room = room_id.clone();
    let by = admin.user_id;
    let task_id = services
        .tasks
        .spawn("quarantine room media", room_id.to_string(), async move {
            let mut found = BTreeSet::new();
            task_services
                .rooms
                .timeline
                .pdus(None, &task_room, None)
                .for_each(|item| {
                    if let Ok((_, pdu)) = item
                        && pdu.kind != TimelineEventType::RoomMember
                        && let Ok(content) = serde_json::from_str::<Value>(pdu.content.get())
                    {
                        collect_mxcs(&content, &mut found);
                    }

                    futures::future::ready(())
                })
                .await;

            let referenced = found.len();
            let quarantined = task_services
                .media
                .quarantine_all(found, &by, body.reason.as_deref())
                .await;

            Ok(json!({ "referenced": referenced, "quarantined": quarantined }))
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

/// # `POST /_phantom/admin/v1/users/{user_id}/media/quarantine`
///
/// Quarantines every file a local user uploaded; answers with how many were
/// not quarantined already.
pub(super) async fn quarantine_user_media(
    State(services): State<RouterState>,
    admin: AdminAuth,
    Path(user_id): Path<OwnedUserId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Reason = super::body(&body)?;

    if !services.server_state.server_is_ours(user_id.server_name()) {
        return Err!(Request(InvalidParam(
            "{user_id} is not a local user; only local uploads have a recorded uploader."
        )));
    }

    let uploads = services.media.uploads_of(&user_id).await;
    let uploaded = uploads.len();
    let quarantined = services
        .media
        .quarantine_all(uploads, &admin.user_id, body.reason.as_deref())
        .await;

    Ok(Json(
        json!({ "uploaded": uploaded, "quarantined": quarantined }),
    ))
}

/// Every string in the JSON that is a valid mxc URI, however deep: `url`,
/// `info.thumbnail_url`, an encrypted file's `file.url` and whatever a custom
/// event names.
fn collect_mxcs(value: &Value, found: &mut BTreeSet<OwnedMxcUri>) {
    match value {
        Value::String(text) if text.starts_with("mxc://") => {
            let mxc = <&MxcUri>::from(text.as_str());
            if mxc.is_valid() {
                found.insert(mxc.to_owned());
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_mxcs(item, found)),
        Value::Object(fields) => fields.values().for_each(|item| collect_mxcs(item, found)),
        _ => {}
    }
}

fn mxc_of(server: &str, media_id: &str) -> Result<OwnedMxcUri> {
    let mxc = OwnedMxcUri::from(format!("mxc://{server}/{media_id}"));
    if !mxc.is_valid() {
        return Err!(Request(InvalidParam("{mxc} is not a valid mxc URI.")));
    }

    Ok(mxc)
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct PurgeHistory {
    /// Purge events sent before this time, in milliseconds since the epoch.
    before_ts_ms: Option<u64>,

    /// Purge events placed before this one.
    before_event_id: Option<OwnedEventId>,

    /// Purge the events local users sent too; they exist nowhere else.
    include_local: bool,
}

/// # `POST /_phantom/admin/v1/rooms/{room_id}/purge_history`
///
/// Deletes the room's timeline events before `before_ts_ms` or
/// `before_event_id`, keeping its state events and latest events, as a
/// tracked task; answers with the task's ID. Events local users sent are
/// kept unless `include_local` is set.
pub(super) async fn purge_history(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: PurgeHistory = super::body(&body)?;

    if !services.rooms.metadata.exists(&room_id).await {
        return Err!(Request(NotFound("The server does not know {room_id}.")));
    }

    let cutoff = match (body.before_ts_ms, &body.before_event_id) {
        (Some(ts), None) => {
            if ts > now_millis() {
                return Err!(Request(InvalidParam(
                    "before_ts_ms is in the future; that would purge the whole history."
                )));
            }

            HistoryCutoff::Timestamp(ts)
        }
        (None, Some(event_id)) => {
            let pdu = services
                .rooms
                .timeline
                .get_pdu(event_id)
                .await
                .map_err(|_| err!(Request(NotFound("{event_id} is not in the timeline."))))?;

            if pdu.room_id != room_id {
                return Err!(Request(InvalidParam("{event_id} is not in {room_id}.")));
            }

            HistoryCutoff::Event(services.rooms.timeline.get_pdu_count(event_id).await?)
        }
        _ => {
            return Err!(Request(InvalidParam(
                "Give either before_ts_ms or before_event_id."
            )));
        }
    };

    if services
        .tasks
        .has_nonterminal("purge room history", room_id.as_str())
    {
        return Err!(Conflict("The room's history is already being purged."));
    }

    let task_services = services.clone();
    let task_room = room_id.clone();
    let task_id = services
        .tasks
        .spawn("purge room history", room_id.to_string(), async move {
            let purged = task_services
                .rooms
                .timeline
                .purge_history(&task_room, cutoff, body.include_local)
                .await?;

            Ok(serde_json::to_value(purged)?)
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum PurgeOrigin {
    Remote,
    Local,
}

#[derive(Deserialize)]
pub(super) struct PurgeMedia {
    origin: PurgeOrigin,
    older_than_days: u64,
}

/// # `POST /_phantom/admin/v1/media/purge`
///
/// Deletes the stored files of an `origin`, "remote" (copies of other
/// servers' media, fetched again if asked for) or "local" (this server's
/// uploads, gone for good), stored more than `older_than_days` ago, as a
/// tracked task; answers with the task's ID.
pub(super) async fn purge_media(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: PurgeMedia = super::body(&body)?;
    let (origin, resource) = match body.origin {
        PurgeOrigin::Remote => (Origin::Remote, "remote"),
        PurgeOrigin::Local => (Origin::Local, "local"),
    };

    if body.older_than_days == 0 && origin == Origin::Local {
        return Err!(Request(InvalidParam(
            "Purging every local upload at once is refused; give at least one day."
        )));
    }

    if services.tasks.has_nonterminal("purge media", resource) {
        return Err!(Conflict("A purge of {resource} media is already running."));
    }

    let before_secs = now_secs().saturating_sub(body.older_than_days.saturating_mul(86_400));
    let task_services = services.clone();
    let task_id = services
        .tasks
        .spawn("purge media", resource.to_owned(), async move {
            let purged = task_services
                .media
                .purge_older_than(origin, before_secs)
                .await?;

            Ok(serde_json::to_value(purged)?)
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::collect_mxcs;

    #[test]
    fn mxcs_are_found_however_deep() {
        let content = json!({
            "body": "a picture of mxc://not.this/one",
            "url": "mxc://a.example/image",
            "info": {
                "thumbnail_url": "mxc://a.example/thumb",
                "thumbnail_file": { "url": "mxc://b.example/encthumb" },
            },
            "file": { "url": "mxc://b.example/enc" },
            "list": ["mxc://c.example/listed", "plain"],
            "broken": "mxc://no-media-id",
        });

        let mut found = BTreeSet::new();
        collect_mxcs(&content, &mut found);
        let found: Vec<&str> = found.iter().map(|mxc| mxc.as_str()).collect();

        assert_eq!(
            found,
            [
                "mxc://a.example/image",
                "mxc://a.example/thumb",
                "mxc://b.example/enc",
                "mxc://b.example/encthumb",
                "mxc://c.example/listed",
            ]
        );
    }
}
