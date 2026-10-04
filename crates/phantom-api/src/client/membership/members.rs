use axum::extract::State;
use futures::{FutureExt, StreamExt};
use phantom_core::{
    Err, Result, at, err, is_equal_to, is_not_equal_to,
    matrix::{PduCount, PduEvent},
    stream::ReadyExt,
};
use phantom_service::{Services, rooms::short::ShortStateHash};
use ruma::{
    RoomId,
    api::Direction,
    api::client::membership::{
        get_member_events,
        joined_members::{self, v3::RoomMember},
    },
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};

use crate::router::Ruma;

/// Lists the room's member events at the current state or a token's snapshot.
///
/// Accepts sync `prev_batch`/`next_batch` and `/messages` tokens.
/// Visibility is decided from the caller's current membership; Synapse
/// decides it from the state at the token.
pub(crate) async fn get_member_events_route(
    State(services): State<crate::router::State>,
    body: Ruma<get_member_events::v3::Request>,
) -> Result<get_member_events::v3::Response> {
    if !services
        .rooms
        .state_accessor
        .user_can_see_state_events(body.sender_user(), &body.room_id)
        .await
    {
        return Err!(Request(Forbidden(
            "You aren't a member of the room and weren't previously a member of the room."
        )));
    }

    let at: Option<PduCount> = body
        .at
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|_| err!(Request(InvalidParam("Invalid `at` token."))))?;

    let shortstatehash = match at {
        None => services
            .rooms
            .state
            .get_room_shortstatehash(&body.room_id)
            .await
            .map_err(|e| err!(Database("Missing state for {:?}: {e:?}", body.room_id)))?,

        Some(at) => shortstatehash_after(&services, &body.room_id, at).await?,
    };

    let membership = body.membership.as_ref();
    let not_membership = body.not_membership.as_ref();
    let membership_filter = |content: &RoomMemberEventContent| {
        membership.is_none_or(is_equal_to!(&content.membership))
            && not_membership.is_none_or(is_not_equal_to!(&content.membership))
    };

    let chunk = services
        .rooms
        .state_accessor
        .state_full(shortstatehash)
        .ready_filter(|((ty, _), _)| *ty == StateEventType::RoomMember)
        .map(at!(1))
        .ready_filter(|pdu| {
            pdu.get_content::<RoomMemberEventContent>()
                .as_ref()
                .is_ok_and(membership_filter)
        })
        .map(PduEvent::into_member_event)
        .collect()
        .boxed()
        .await;

    Ok(get_member_events::v3::Response::new(chunk))
}

/// # `GET /_matrix/client/r0/rooms/{roomId}/joined_members`
///
/// Lists all members of a room.
///
/// - The sender user must be in the room
/// - TODO: An appservice just needs a puppet joined
pub(crate) async fn joined_members_route(
    State(services): State<crate::router::State>,
    body: Ruma<joined_members::v3::Request>,
) -> Result<joined_members::v3::Response> {
    let can_peek = services
        .rooms
        .state_cache
        .is_joined(body.sender_user(), &body.room_id)
        .await
        || services
            .rooms
            .state_accessor
            .is_world_readable(&body.room_id)
            .await;

    if !can_peek {
        return Err!(Request(Forbidden("You aren't a member of the room.")));
    }

    let joined = services
        .rooms
        .state_accessor
        .room_state_full(&body.room_id)
        .ready_filter_map(Result::ok)
        .ready_filter(|((ty, _), _)| *ty == StateEventType::RoomMember)
        .map(at!(1))
        .ready_filter_map(|pdu| {
            let content = pdu.get_content::<RoomMemberEventContent>().ok()?;

            let matches = content.membership == MembershipState::Join;

            matches.then(|| {
                let sender = pdu.sender.clone();
                let mut member = RoomMember::new();
                member.display_name = content.displayname;
                member.avatar_url = content.avatar_url;

                (sender, member)
            })
        })
        .collect()
        .boxed()
        .await;

    Ok(joined_members::v3::Response::new(joined))
}

/// The room's state just after the timeline position `at`: the state before
/// the next event, or the current state when nothing follows.
async fn shortstatehash_after(
    services: &Services,
    room_id: &RoomId,
    at: PduCount,
) -> Result<ShortStateHash> {
    let next = services
        .rooms
        .timeline
        .pdus(None, room_id, Some(at.saturating_inc(Direction::Forward)))
        .ready_filter_map(Result::ok)
        .boxed()
        .next()
        .await;

    match next {
        Some((_, pdu)) => {
            services
                .rooms
                .state_accessor
                .pdu_shortstatehash(&pdu.event_id)
                .await
        }
        None => services
            .rooms
            .state
            .get_room_shortstatehash(room_id)
            .await
            .map_err(|e| err!(Request(NotFound("Room {room_id:?} not found: {e:?}")))),
    }
}
