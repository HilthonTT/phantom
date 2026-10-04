use axum::extract::State;
use futures::{FutureExt, StreamExt, TryFutureExt};
use phantom_core::{Err, Result, debug_warn, err, stream::IterStream, trace};
use phantom_service::{
    Services,
    net::federation::feds::{Fault, Opts, OutcomeExt, Record},
};
use ruma::{
    OwnedRoomId, OwnedServerName, RoomId, UserId,
    api::{client::room::get_summary, federation::space::get_hierarchy},
    events::room::member::MembershipState,
    room::{JoinRuleSummary, RoomSummary},
};

use crate::router::{ClientIp, Ruma, RumaResponse};

/// # `GET /_matrix/client/unstable/im.nheko.summary/rooms/{roomIdOrAlias}/summary`
///
/// Returns a short description of the state of a room.
///
/// This is the "wrong" endpoint that some implementations/clients may use
/// according to the MSC. Request and response bodies are the same as
/// `get_room_summary`.
///
/// An implementation of [MSC3266](https://github.com/matrix-org/matrix-spec-proposals/pull/3266)
pub(crate) async fn get_room_summary_legacy(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_summary::v1::Request>,
) -> Result<RumaResponse<get_summary::v1::Response>> {
    get_room_summary(State(services), ClientIp(client), body)
        .boxed()
        .await
        .map(RumaResponse)
}

/// # `GET /_matrix/client/unstable/im.nheko.summary/summary/{roomIdOrAlias}`
///
/// Returns a short description of the state of a room.
///
/// An implementation of [MSC3266](https://github.com/matrix-org/matrix-spec-proposals/pull/3266)
#[tracing::instrument(skip_all, fields(%client), name = "room_summary")]
pub(crate) async fn get_room_summary(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<get_summary::v1::Request>,
) -> Result<get_summary::v1::Response> {
    let (room_id, servers) = services
        .rooms
        .alias
        .maybe_resolve_with_servers(&body.room_id_or_alias, Some(&body.via))
        .await?;

    if services.rooms.metadata.is_banned(&room_id).await {
        return Err!(Request(Forbidden(
            "This room is banned on this homeserver."
        )));
    }

    room_summary_response(&services, &room_id, &servers, body.sender_user.as_deref())
        .boxed()
        .await
}

async fn room_summary_response(
    services: &Services,
    room_id: &RoomId,
    servers: &[OwnedServerName],
    sender_user: Option<&UserId>,
) -> Result<get_summary::v1::Response> {
    if services
        .rooms
        .state_cache
        .server_in_room(services.server_state.server_name(), room_id)
        .await
    {
        return local_room_summary_response(services, room_id, sender_user)
            .boxed()
            .await;
    }

    let summary =
        remote_room_summary_hierarchy_response(services, room_id, servers, sender_user).await?;

    let mut response = get_summary::v1::Response::new(summary);
    response.membership = sender_user.is_some().then_some(MembershipState::Leave);

    Ok(response)
}

async fn local_room_summary_response(
    services: &Services,
    room_id: &RoomId,
    sender_user: Option<&UserId>,
) -> Result<get_summary::v1::Response> {
    trace!(
        ?sender_user,
        "Sending local room summary response for {room_id:?}"
    );
    let summary = services.rooms.state_accessor.room_summary(room_id).await;

    trace!(?summary.join_rule, summary.world_readable, summary.guest_can_join);
    user_can_see_summary(
        services,
        room_id,
        &summary.join_rule,
        summary.guest_can_join,
        summary.world_readable,
        sender_user,
    )
    .await?;

    let membership = match sender_user {
        Some(sender_user) => Some(
            services
                .rooms
                .state_accessor
                .get_member(room_id, sender_user)
                .map_ok_or_else(|_| MembershipState::Leave, |content| content.membership)
                .await,
        ),
        None => None,
    };

    let mut response = get_summary::v1::Response::new(summary);
    response.membership = membership;

    Ok(response)
}

/// used by MSC3266 to fetch a room's info if we do not know about it
async fn remote_room_summary_hierarchy_response(
    services: &Services,
    room_id: &RoomId,
    servers: &[OwnedServerName],
    sender_user: Option<&UserId>,
) -> Result<RoomSummary> {
    trace!(
        ?sender_user,
        ?servers,
        "Sending remote room summary response for {room_id:?}"
    );
    if !services.config.federation.allow_federation {
        return Err!(Request(Forbidden("Federation is disabled.")));
    }

    if services.rooms.metadata.is_disabled(room_id).await {
        return Err!(Request(Forbidden(
            "Federation of room {room_id} is currently disabled on this server."
        )));
    }

    if servers.is_empty() {
        return Err!(Request(NotFound(
            "Room is unknown to this server and no servers were provided to fetch it over \
             federation."
        )));
    }

    let request = get_hierarchy::v1::Request::new(room_id.to_owned());
    let opts = Opts {
        record: Record::Contribute,
        ..Default::default()
    };
    let acceptable = |response: &get_hierarchy::v1::Response| {
        trace!(?response, "federation response");
        let returned_room_id = &response.room.summary.room_id;
        let accepted = returned_room_id == room_id;

        if !accepted {
            debug_warn!(
                message = format_args!("federation room hierarchy response did not match request"),
                %returned_room_id,
                requested_room_id = %room_id
            );
        }

        accepted
    };

    let response = services
        .federation
        .fanout_to(
            servers.iter().cloned().stream(),
            move |_| request.clone(),
            opts,
        )
        .inspect(|outcome| match &outcome.result {
            Ok(_) => {}
            Err(Fault::Error(e)) => {
                debug_warn!(?e, "Failed to fetch room hierarchy over federation");
            }
            Err(fault) => {
                debug_warn!(?fault, "Failed to fetch room hierarchy over federation");
            }
        })
        .first_acceptable(acceptable)
        .await;

    let Some((_, response)) = response else {
        return Err!(Request(NotFound(
            "Room is unknown to this server and was unable to fetch over federation with the \
             provided servers available"
        )));
    };

    let room = response.room;
    let summary = &room.summary;

    user_can_see_summary(
        services,
        room_id,
        &summary.join_rule,
        summary.guest_can_join,
        summary.world_readable,
        sender_user,
    )
    .await
    .map(|()| room.summary)
}

async fn user_can_see_summary(
    services: &Services,
    room_id: &RoomId,
    join_rule: &JoinRuleSummary,
    guest_can_join: bool,
    world_readable: bool,
    sender_user: Option<&UserId>,
) -> Result {
    let is_public_room = matches!(
        join_rule,
        JoinRuleSummary::Public | JoinRuleSummary::Knock | JoinRuleSummary::KnockRestricted(_)
    );

    if is_public_room {
        return Ok(());
    }

    let Some(sender_user) = sender_user else {
        return world_readable.then_some(()).ok_or_else(|| {
            err!(Request(Forbidden(
                "Room is not world readable or publicly accessible/joinable, authentication is \
                 required"
            )))
        });
    };

    if services
        .rooms
        .state_accessor
        .user_can_see_state_events(sender_user, room_id)
        .await
    {
        return Ok(());
    }

    // Guest accounts carry no password, which `is_deactivated` reports as true.
    if guest_can_join
        && services
            .users
            .is_deactivated(sender_user)
            .await
            .unwrap_or(false)
    {
        return Ok(());
    }

    let allowed_room_ids: &[OwnedRoomId] = match join_rule {
        JoinRuleSummary::Restricted(restricted) => &restricted.allowed_room_ids,
        _ => &[],
    };

    if allowed_room_ids
        .iter()
        .stream()
        .any(|allowed| services.rooms.state_cache.is_joined(sender_user, allowed))
        .await
    {
        return Ok(());
    }

    Err!(Request(Forbidden(
        "Room is not world readable, not publicly accessible/joinable, restricted room \
         conditions not met, and guest access is forbidden. Not allowed to see details of this \
         room."
    )))
}
