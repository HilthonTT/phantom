use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    response::IntoResponse,
};
use futures::{StreamExt, future::join_all};
use phantom_core::{Err, Result};
use ruma::{OwnedRoomAliasId, OwnedRoomId, RoomId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::router::{AdminAuth, State as RouterState};

#[derive(Serialize)]
pub(super) struct Room {
    room_id: OwnedRoomId,
    name: Option<String>,
    canonical_alias: Option<OwnedRoomAliasId>,
    topic: Option<String>,

    /// None when the server knows the room but holds none of its state, as
    /// for a room it was only invited to.
    version: Option<String>,

    joined_members: u64,
    local_members: usize,

    encrypted: bool,
    join_rule: String,

    /// Listed in this server's public room directory.
    published: bool,
    banned: bool,
    disabled: bool,
}

/// # `GET /_phantom/admin/v1/rooms`
///
/// Every room the server knows of.
pub(super) async fn rooms(
    State(services): State<RouterState>,
    _admin: AdminAuth,
) -> Result<impl IntoResponse> {
    let ids: Vec<OwnedRoomId> = services
        .rooms
        .metadata
        .iter_ids()
        .map(ToOwned::to_owned)
        .collect()
        .await;

    let rooms = join_all(ids.iter().map(|room_id| room(&services, room_id))).await;

    Ok(Json(rooms))
}

async fn room(services: &RouterState, room_id: &RoomId) -> Room {
    let rooms = &services.rooms;

    Room {
        room_id: room_id.to_owned(),
        name: rooms.state_accessor.get_name(room_id).await.ok(),
        canonical_alias: rooms.state_accessor.get_canonical_alias(room_id).await.ok(),
        topic: rooms.state_accessor.get_room_topic(room_id).await.ok(),
        version: rooms
            .state
            .get_room_version(room_id)
            .await
            .ok()
            .map(|version| version.to_string()),
        joined_members: rooms
            .state_cache
            .room_joined_count(room_id)
            .await
            .unwrap_or(0),
        local_members: rooms.state_cache.local_users_in_room(room_id).count().await,
        encrypted: rooms.state_accessor.is_encrypted_room(room_id).await,
        join_rule: rooms
            .state_accessor
            .get_join_rules(room_id)
            .await
            .as_str()
            .to_owned(),
        published: rooms.directory.is_public_room(room_id).await,
        banned: rooms.metadata.is_banned(room_id).await,
        disabled: rooms.metadata.is_disabled(room_id).await,
    }
}

/// # `PUT /_phantom/admin/v1/rooms/{room_id}/ban`
///
/// Bans a room: local users can no longer join it, and the server stops
/// taking part in it over federation.
pub(super) async fn ban(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    not_the_admin_room(&services, &room_id).await?;
    services.rooms.metadata.ban_room(&room_id, true);

    Ok(Json(json!({})))
}

/// # `DELETE /_phantom/admin/v1/rooms/{room_id}/ban`
pub(super) async fn unban(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    services.rooms.metadata.ban_room(&room_id, false);

    Ok(Json(json!({})))
}

/// # `POST /_phantom/admin/v1/rooms/{room_id}/shutdown`
///
/// Evicts every local member, frees the room's local aliases and takes it
/// out of the directory, as a tracked task; answers with the task's ID.
pub(super) async fn shutdown(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
) -> Result<impl IntoResponse> {
    not_the_admin_room(&services, &room_id).await?;
    known(&services, &room_id).await?;

    let task_services = services.clone();
    let task_room = room_id.clone();
    let task_id = services
        .tasks
        .spawn("shutdown room", room_id.to_string(), async move {
            let rooms = &task_services.rooms;
            let state_lock = rooms.state.mutex.lock(&*task_room).await;
            let summary = rooms.delete.shutdown_room(&task_room, &state_lock).await;

            Ok(serde_json::to_value(summary)?)
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct Delete {
    /// Purge even what the room's state cannot account for.
    force: bool,
}

/// # `DELETE /_phantom/admin/v1/rooms/{room_id}`
///
/// Shuts the room down, then purges everything the server holds of it, as a
/// tracked task; answers with the task's ID.
pub(super) async fn delete(
    State(services): State<RouterState>,
    _admin: AdminAuth,
    Path(room_id): Path<OwnedRoomId>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let body: Delete = super::body(&body)?;
    not_the_admin_room(&services, &room_id).await?;
    known(&services, &room_id).await?;

    let task_services = services.clone();
    let task_room = room_id.clone();
    let task_id = services
        .tasks
        .spawn("delete room", room_id.to_string(), async move {
            let rooms = &task_services.rooms;
            let state_lock = rooms.state.mutex.lock(&*task_room).await;
            let summary = rooms
                .delete
                .delete_room(&task_room, body.force, &state_lock)
                .await?;

            Ok(serde_json::to_value(summary)?)
        });

    Ok(Json(json!({ "task_id": task_id.as_str() })))
}

async fn not_the_admin_room(services: &RouterState, room_id: &RoomId) -> Result {
    if services.admin.is_admin_room(room_id).await {
        return Err!(Request(Forbidden(
            "The admin room cannot be banned, shut down or deleted."
        )));
    }

    Ok(())
}

async fn known(services: &RouterState, room_id: &RoomId) -> Result {
    if !services.rooms.metadata.exists(room_id).await {
        return Err!(Request(NotFound("The server does not know {room_id}.")));
    }

    Ok(())
}
