use std::cmp;

use axum::extract::State;
use futures::{
    FutureExt, StreamExt, TryFutureExt,
    future::{join, join4, join5},
};
use phantom_core::{
    Err, Error, Result, err, future::TryExt as TryFutureExtExt, info, math::Expected,
    matrix::Event, stream::ReadyExt, stream::WidebandExt, warn,
};
use phantom_service::{Services, ops::moderation::Restriction};
use ruma::{
    OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, ServerName, UInt, UserId,
    api::{
        client::{
            directory::{
                get_public_rooms, get_public_rooms_filtered, get_room_visibility,
                set_room_visibility,
            },
            room,
        },
        federation,
    },
    directory::{Filter, PublicRoomsChunk, PublicRoomsChunkInit, RoomNetwork, RoomTypeFilter},
    events::StateEventType,
    uint,
};

use crate::router::{ClientIp, Ruma};

/// # `POST /_matrix/client/v3/publicRooms`
///
/// Lists the public rooms on this server.
///
/// - Rooms are ordered by the number of joined members
#[tracing::instrument(skip_all, fields(%client), name = "publicrooms")]
pub(crate) async fn get_public_rooms_filtered_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_public_rooms_filtered::v3::Request>,
) -> Result<get_public_rooms_filtered::v3::Response> {
    check_server_banned(&services, body.server.as_deref())?;

    get_public_rooms_filtered_helper(
        &services,
        body.server.as_deref(),
        body.limit,
        body.since.as_deref(),
        &body.filter,
        &body.room_network,
    )
    .map_err(|e| mask_remote_failure(&services, body.server.as_deref(), e))
    .await
}

/// # `GET /_matrix/client/v3/publicRooms`
///
/// Lists the public rooms on this server.
///
/// - Rooms are ordered by the number of joined members
#[tracing::instrument(skip_all, fields(%client), name = "publicrooms")]
pub(crate) async fn get_public_rooms_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_public_rooms::v3::Request>,
) -> Result<get_public_rooms::v3::Response> {
    check_server_banned(&services, body.server.as_deref())?;

    let response = get_public_rooms_filtered_helper(
        &services,
        body.server.as_deref(),
        body.limit,
        body.since.as_deref(),
        &Filter::default(),
        &RoomNetwork::Matrix,
    )
    .map_err(|e| mask_remote_failure(&services, body.server.as_deref(), e))
    .await?;

    let mut public_rooms = get_public_rooms::v3::Response::new(response.chunk);
    public_rooms.prev_batch = response.prev_batch;
    public_rooms.next_batch = response.next_batch;
    public_rooms.total_room_count_estimate = response.total_room_count_estimate;

    Ok(public_rooms)
}

/// # `PUT /_matrix/client/r0/directory/list/room/{roomId}`
///
/// Sets the visibility of a given room in the room directory.
#[tracing::instrument(skip_all, fields(%client), name = "room_directory")]
pub(crate) async fn set_room_visibility_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<set_room_visibility::v3::Request>,
) -> Result<set_room_visibility::v3::Response> {
    let sender_user = body.sender_user();

    if !services.rooms.metadata.exists(&body.room_id).await {
        // Return 404 if the room doesn't exist
        return Err!(Request(NotFound("Room not found")));
    }

    if services
        .users
        .is_deactivated(sender_user)
        .await
        .unwrap_or(false)
        && body.appservice_info.is_none()
    {
        return Err!(Request(Forbidden(
            "Guests cannot publish to room directories"
        )));
    }

    if !user_can_publish_room(&services, sender_user, &body.room_id).await? {
        return Err!(Request(Forbidden(
            "User is not allowed to publish this room"
        )));
    }

    match &body.visibility {
        room::Visibility::Public => {
            if services.server.config.client.lockdown_public_room_directory
                && !services.admin.user_is_admin(sender_user).await
                && body.appservice_info.is_none()
            {
                info!(
                    "Non-admin user {sender_user} tried to publish {0} to the room directory \
                     while \"lockdown_public_room_directory\" is enabled",
                    body.room_id
                );

                return Err!(Request(Forbidden(
                    "Publishing rooms to the room directory is not allowed"
                )));
            }

            services.rooms.directory.set_public(&body.room_id)?;

            info!(
                "{sender_user} made {0} public to the room directory",
                body.room_id
            );
        }
        room::Visibility::Private => services.rooms.directory.set_not_public(&body.room_id)?,
        _ => {
            return Err!(Request(InvalidParam(
                "Room visibility type is not supported."
            )));
        }
    }

    Ok(set_room_visibility::v3::Response::new())
}

/// # `GET /_matrix/client/r0/directory/list/room/{roomId}`
///
/// Gets the visibility of a given room in the room directory.
pub(crate) async fn get_room_visibility_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_room_visibility::v3::Request>,
) -> Result<get_room_visibility::v3::Response> {
    if !services.rooms.metadata.exists(&body.room_id).await {
        // Return 404 if the room doesn't exist
        return Err!(Request(NotFound("Room not found")));
    }

    let visibility = if services.rooms.directory.is_public_room(&body.room_id).await {
        room::Visibility::Public
    } else {
        room::Visibility::Private
    };

    Ok(get_room_visibility::v3::Response::new(visibility))
}

pub(crate) async fn get_public_rooms_filtered_helper(
    services: &Services,
    server: Option<&ServerName>,
    limit: Option<UInt>,
    since: Option<&str>,
    filter: &Filter,
    _network: &RoomNetwork,
) -> Result<get_public_rooms_filtered::v3::Response> {
    if let Some(other_server) = remote_server(services, server) {
        let mut request = federation::directory::get_public_rooms_filtered::v1::Request::new();
        request.limit = limit;
        request.since = since.map(ToOwned::to_owned);
        request.filter = filter.clone();
        request.room_network = RoomNetwork::Matrix;

        let response = services.federation.execute(other_server, request).await?;

        let mut public_rooms = get_public_rooms_filtered::v3::Response::new();
        public_rooms.chunk = response.chunk;
        public_rooms.prev_batch = response.prev_batch;
        public_rooms.next_batch = response.next_batch;
        public_rooms.total_room_count_estimate = response.total_room_count_estimate;

        return Ok(public_rooms);
    }

    // Use limit or else 10, with maximum 100
    let limit: usize = limit.map_or(10_u64, u64::from).try_into()?;
    let mut num_since: usize = 0;

    if let Some(s) = &since {
        let mut characters = s.chars();
        let backwards = match characters.next() {
            Some('n') => false,
            Some('p') => true,
            _ => {
                return Err!(Request(InvalidParam("Invalid `since` token")));
            }
        };

        num_since = characters
            .collect::<String>()
            .parse()
            .map_err(|_| err!(Request(InvalidParam("Invalid `since` token."))))?;

        if backwards {
            num_since = num_since.saturating_sub(limit);
        }
    }

    let search_term = filter.generic_search_term.as_deref().map(str::to_lowercase);

    let search_room_id = filter
        .generic_search_term
        .as_deref()
        .filter(|_| services.config.client.allow_public_room_search_by_id)
        .filter(|s| s.starts_with('!'))
        .filter(|s| s.len() > 5); // require some characters to limit scope.

    let mut all_rooms: Vec<PublicRoomsChunk> = services
        .rooms
        .directory
        .public_rooms()
        .map(ToOwned::to_owned)
        .wide_then(|room_id| public_rooms_chunk(services, room_id))
        .ready_filter_map(|chunk| {
            if !filter.room_types.is_empty()
                && !filter
                    .room_types
                    .contains(&RoomTypeFilter::from(chunk.room_type.clone()))
            {
                return None;
            }

            if let Some(query) = search_room_id
                && chunk.room_id.as_str().contains(query)
            {
                return Some(chunk);
            }

            if let Some(query) = search_term.as_deref() {
                if let Some(name) = &chunk.name
                    && name.as_str().to_lowercase().contains(query)
                {
                    return Some(chunk);
                }

                if let Some(topic) = &chunk.topic
                    && topic.to_lowercase().contains(query)
                {
                    return Some(chunk);
                }

                if let Some(canonical_alias) = &chunk.canonical_alias
                    && canonical_alias.as_str().to_lowercase().contains(query)
                {
                    return Some(chunk);
                }

                return None;
            }

            // No search term
            Some(chunk)
        })
        // We need to collect all, so we can sort by member count
        .collect()
        .await;

    all_rooms.sort_by_key(|r| cmp::Reverse(r.num_joined_members));

    let total_room_count_estimate = UInt::try_from(all_rooms.len())
        .unwrap_or_else(|_| uint!(0))
        .into();

    let chunk: Vec<_> = all_rooms.into_iter().skip(num_since).take(limit).collect();

    let prev_batch = num_since.ne(&0).then_some(format!("p{num_since}"));

    let next_batch = chunk
        .len()
        .ge(&limit)
        .then_some(format!("n{}", num_since.expected_add(limit)));

    let mut public_rooms = get_public_rooms_filtered::v3::Response::new();
    public_rooms.chunk = chunk;
    public_rooms.prev_batch = prev_batch;
    public_rooms.next_batch = next_batch;
    public_rooms.total_room_count_estimate = total_room_count_estimate;

    Ok(public_rooms)
}

/// Check whether the user can publish to the room directory via power levels of
/// room history visibility event or room creator
async fn user_can_publish_room(
    services: &Services,
    user_id: &UserId,
    room_id: &RoomId,
) -> Result<bool> {
    match services
        .rooms
        .state_accessor
        .get_power_levels(room_id)
        .await
    {
        Ok(power_levels) => {
            Ok(power_levels.user_can_send_state(user_id, StateEventType::RoomHistoryVisibility))
        }
        _ => {
            match services
                .rooms
                .state_accessor
                .room_state_get(room_id, &StateEventType::RoomCreate, "")
                .await
            {
                Ok(event) => Ok(event.sender() == user_id),
                _ => Err!(Request(Forbidden(
                    "User is not allowed to publish this room"
                ))),
            }
        }
    }
}

async fn public_rooms_chunk(services: &Services, room_id: OwnedRoomId) -> PublicRoomsChunk {
    let name = services.rooms.state_accessor.get_name(&room_id).ok();

    let room_type = services.rooms.state_accessor.get_room_type(&room_id).ok();

    let canonical_alias = directory_alias(services, &room_id);

    let avatar_url = services
        .rooms
        .state_accessor
        .get_avatar(&room_id)
        .map(|content| content.into_option().and_then(|content| content.url));

    let topic = services.rooms.state_accessor.get_room_topic(&room_id).ok();

    let world_readable = services.rooms.state_accessor.is_world_readable(&room_id);

    let join_rule = services
        .rooms
        .state_accessor
        .get_join_rules(&room_id)
        .map(|join_rule| join_rule.kind());

    let guest_can_join = services.rooms.state_accessor.guest_can_join(&room_id);

    let num_joined_members = services
        .rooms
        .state_cache
        .room_joined_count(&room_id)
        .map(|x| {
            x.ok()
                .and_then(|x| x.try_into().ok())
                .unwrap_or_else(|| uint!(0))
        });

    let (
        (avatar_url, canonical_alias, guest_can_join, join_rule, name),
        (num_joined_members, room_type, topic, world_readable),
    ) = join(
        join5(avatar_url, canonical_alias, guest_can_join, join_rule, name),
        join4(num_joined_members, room_type, topic, world_readable),
    )
    .boxed()
    .await;

    let mut chunk = PublicRoomsChunk::from(PublicRoomsChunkInit {
        num_joined_members,
        room_id,
        world_readable,
        guest_can_join,
    });
    chunk.avatar_url = avatar_url;
    chunk.canonical_alias = canonical_alias;
    chunk.join_rule = join_rule;
    chunk.name = name;
    chunk.room_type = room_type;
    chunk.topic = topic;

    chunk
}

/// Alias for the room's directory entry: the room's canonical alias while it
/// still resolves to the room.
async fn directory_alias(services: &Services, room_id: &RoomId) -> Option<OwnedRoomAliasId> {
    let alias = services
        .rooms
        .state_accessor
        .get_canonical_alias(room_id)
        .await
        .ok()?;

    alias_resolves_to(services, &alias, room_id)
        .await
        .then_some(alias)
}

async fn alias_resolves_to(services: &Services, alias: &RoomAliasId, room_id: &RoomId) -> bool {
    services.server_state.alias_is_local(alias)
        && services
            .rooms
            .alias
            .resolve_local_alias(alias)
            .await
            .is_ok_and(|resolved| resolved == room_id)
}

fn check_server_banned(services: &Services, server: Option<&ServerName>) -> Result {
    let Some(server) = server else {
        return Ok(());
    };

    if services
        .moderation
        .forbids(server, Restriction::RoomDirectory)
    {
        return Err!(Request(Forbidden("Server is banned on this homeserver.")));
    }

    Ok(())
}

/// Masks a remote directory failure behind a generic gateway error.
///
/// The remote chooses its own error, so forwarding one verbatim lets a third
/// party pick what our client sees. A query served locally contacts nobody, so
/// its error is returned unchanged rather than relabelled as an upstream
/// failure.
fn mask_remote_failure(services: &Services, server: Option<&ServerName>, error: Error) -> Error {
    let Some(server) = remote_server(services, server) else {
        return error;
    };

    warn!(%server, %error, "Failed to query remote public rooms directory");

    err!(Request(ConnectionFailed(
        "Unable to query the remote public rooms directory."
    )))
}

/// The server a directory query is routed to, when that server is not us.
///
/// Routing the query and masking its failure both read this, so the set of
/// requests that reach a third party cannot drift from the set whose errors are
/// masked.
fn remote_server<'a>(
    services: &Services,
    server: Option<&'a ServerName>,
) -> Option<&'a ServerName> {
    server.filter(|server| !services.server_state.server_is_ours(server))
}
