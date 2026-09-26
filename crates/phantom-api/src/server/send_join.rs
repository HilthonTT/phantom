use std::{borrow::Borrow, collections::BTreeSet, sync::Arc};

use axum::extract::State;
use futures::{
    FutureExt, StreamExt, TryStreamExt,
    future::{ready, try_join4},
};
use phantom_core::{
    Result, debug_error, err,
    stream::{BroadbandExt, IterStream, TryBroadbandExt},
};
use phantom_service::Services;
use ruma::{
    EventId, OwnedEventId, OwnedUserId, RoomId, RoomVersionId, ServerName, UserId,
    api::federation::membership::create_join_event,
    events::{StateEventType, room::member::MembershipState},
};
use serde_json::value::RawValue as RawJsonValue;

use super::{
    access::require_known_room,
    membership::{accept_timeline_event, parse_membership_event, reject_forbidden_room_server},
    restricted_join::validate_authorising_user,
};
use crate::router::Ruma;

pub(crate) async fn create_join_event_v2_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_join_event::v2::Request>,
) -> Result<create_join_event::v2::Response> {
    let room_id: &RoomId = &body.room_id;
    let origin = body.origin();
    let omit_members = body.omit_members;

    reject_forbidden_room_server(&services, origin, room_id)?;

    services.sending.notify_peer_alive(origin).await;

    let servers_in_room = if omit_members {
        Some(servers_in_room(&services, room_id).await)
    } else {
        None
    };

    let mut room_state = accept_join(&services, origin, room_id, &body.pdu, omit_members)
        .boxed()
        .await?;

    room_state.members_omitted = omit_members;
    room_state.servers_in_room = servers_in_room;

    Ok(create_join_event::v2::Response::new(room_state))
}

async fn servers_in_room(services: &Services, room_id: &RoomId) -> Vec<String> {
    services
        .rooms
        .state_cache
        .room_servers(room_id)
        .map(ToString::to_string)
        .collect()
        .await
}

async fn accept_join(
    services: &Services,
    origin: &ServerName,
    room_id: &RoomId,
    pdu: &RawJsonValue,
    omit_members: bool,
) -> Result<create_join_event::v2::RoomState> {
    require_known_room(services, room_id, origin).await?;

    let shortstatehash = services
        .rooms
        .state
        .get_room_shortstatehash(room_id)
        .await
        .map_err(|e| err!(Request(NotFound(error!("Room has no state: {e}")))))?;

    let room_version = services.rooms.state.get_room_version(room_id).await?;
    let rules = room_version.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "Unsupported room version {room_version}."
        )))
    })?;

    let join = parse_membership_event(
        services,
        origin,
        room_id,
        &room_version,
        pdu,
        MembershipState::Join,
    )
    .await?;

    let joining_user = join.sender;
    let mut value = join.value;

    if let Some(authorising_user) = &join.content.join_authorized_via_users_server {
        validate_authorising_user(services, authorising_user, &joining_user, room_id, &rules)
            .await?;
    }

    services
        .server_keys
        .hash_and_sign_event(&mut value, &room_version)
        .map_err(|e| {
            err!(Request(InvalidParam(warn!(
                "Failed to sign send_join event: {e}"
            ))))
        })?;

    let heroes = if omit_members {
        room_heroes(services, room_id, shortstatehash, &joining_user).await
    } else {
        Vec::new()
    };

    let kept_members: Arc<BTreeSet<OwnedUserId>> =
        Arc::new(heroes.into_iter().chain([joining_user.clone()]).collect());

    let state_ids = services
        .rooms
        .state_accessor
        .state_full_ids(shortstatehash)
        .broad_filter_map(move |(shortstatekey, event_id): (_, OwnedEventId)| {
            let kept_members = kept_members.clone();

            async move {
                if omit_members
                    && let Ok((kind, state_key)) = services
                        .rooms
                        .short
                        .get_statekey_from_short(shortstatekey)
                        .await
                    && kind == StateEventType::RoomMember
                    && let Ok(user_id) = <&UserId>::try_from(state_key.as_str())
                    && !kept_members.contains(user_id)
                {
                    return None;
                }

                Some(event_id)
            }
        })
        .collect::<Vec<_>>();

    let pdu_id =
        accept_timeline_event(services, origin, room_id, &join.event_id, value.clone()).await?;

    let mut state_ids = state_ids.await;
    state_ids.sort_unstable();

    let include_auth_event =
        |event_id: &OwnedEventId| !omit_members || state_ids.binary_search(event_id).is_err();

    let auth_chain = services
        .rooms
        .auth_chain
        .event_ids_iter(room_id, state_ids.iter().map(Borrow::borrow))
        .try_filter(|event_id| ready(include_auth_event(event_id)))
        .broad_and_then(async |event_id| federation_pdu(services, &event_id, &room_version).await)
        .try_collect();

    let state = state_ids
        .iter()
        .try_stream()
        .broad_and_then(async |event_id| federation_pdu(services, event_id, &room_version).await)
        .try_collect();

    let event = services
        .federation
        .format_pdu(value, Some(&room_version))
        .map(Some)
        .map(Ok);

    let broadcast = services.sending.send_pdu_room(room_id, &pdu_id);

    let (auth_chain, state, event, ()) = try_join4(auth_chain, state, event, broadcast)
        .boxed()
        .await?;

    let mut room_state = create_join_event::v2::RoomState::new();
    room_state.auth_chain = auth_chain;
    room_state.state = state;
    room_state.event = event;

    Ok(room_state)
}

async fn room_heroes(
    services: &Services,
    room_id: &RoomId,
    shortstatehash: u64,
    joining_user: &UserId,
) -> Vec<OwnedUserId> {
    let state_accessor = &services.rooms.state_accessor;

    let has_name = state_accessor
        .state_contains(shortstatehash, &StateEventType::RoomName, "")
        .await;

    let has_alias = state_accessor
        .state_contains(shortstatehash, &StateEventType::RoomCanonicalAlias, "")
        .await;

    if has_name || has_alias {
        return Vec::new();
    }

    services
        .rooms
        .state_cache
        .heroes(room_id, joining_user)
        .await
}

async fn federation_pdu(
    services: &Services,
    event_id: &EventId,
    room_version: &RoomVersionId,
) -> Result<Box<RawJsonValue>> {
    let pdu = services
        .rooms
        .timeline
        .get_pdu_json(event_id)
        .await
        .inspect_err(|e| debug_error!("Event {event_id} for send_join response not found: {e}"))?;

    Ok(services
        .federation
        .format_pdu(pdu, Some(room_version))
        .await)
}
