use axum::{Json, extract::State, response::IntoResponse};
use futures::{StreamExt, future::join_all};
use phantom_core::Result;
use ruma::{OwnedRoomAliasId, OwnedRoomId, RoomId};
use serde::Serialize;

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
