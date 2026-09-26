use futures::StreamExt;
use phantom_core::{Err, Result, debug_info, stream::IterStream};
use phantom_service::Services;
use ruma::{
    RoomId, UserId,
    events::{
        StateEventType,
        room::join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent},
    },
    room_version_rules::RoomVersionRules,
};

pub(super) async fn requires_authorising_user(
    services: &Services,
    user_id: &UserId,
    room_id: &RoomId,
    rules: &RoomVersionRules,
) -> Result<bool> {
    if !rules.authorization.restricted_join_rule {
        return Ok(false);
    }

    let state_cache = &services.rooms.state_cache;
    if state_cache.is_joined(user_id, room_id).await
        || state_cache.is_invited(user_id, room_id).await
    {
        return Ok(false);
    }

    let Ok(join_rules) = services
        .rooms
        .state_accessor
        .room_state_get_content::<RoomJoinRulesEventContent>(
            room_id,
            &StateEventType::RoomJoinRules,
            "",
        )
        .await
    else {
        return Ok(false);
    };

    let (JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted)) =
        join_rules.join_rule
    else {
        return Ok(false);
    };

    if restricted.allow.is_empty() {
        debug_info!("{room_id} is restricted but the allow key is empty");
        return Ok(false);
    }

    let in_allowed_room = restricted
        .allow
        .iter()
        .filter_map(|rule| match rule {
            AllowRule::RoomMembership(membership) => Some(&membership.room_id),
            _ => None,
        })
        .stream()
        .any(|allowed_room| state_cache.is_joined(user_id, allowed_room))
        .await;

    if !in_allowed_room {
        return Err!(Request(UnableToAuthorizeJoin(
            "Joining user is not known to be in any required room."
        )));
    }

    Ok(true)
}

pub(super) async fn validate_authorising_user(
    services: &Services,
    authorising_user: &UserId,
    joining_user: &UserId,
    room_id: &RoomId,
    rules: &RoomVersionRules,
) -> Result {
    if !rules.authorization.restricted_join_rule {
        return Err!(Request(InvalidParam(
            "Room version does not support restricted rooms but join_authorised_via_users_server \
             ({authorising_user}) was found in the event."
        )));
    }

    if !services.server_state.user_is_local(authorising_user) {
        return Err!(Request(InvalidParam(
            "Cannot authorise membership event through {authorising_user} as they do not belong \
             to this homeserver."
        )));
    }

    if !services
        .rooms
        .state_cache
        .is_joined(authorising_user, room_id)
        .await
    {
        return Err!(Request(InvalidParam(
            "Authorising user {authorising_user} is not in the room you are trying to join, they \
             cannot authorise your join."
        )));
    }

    if !requires_authorising_user(services, joining_user, room_id, rules).await? {
        return Err!(Request(UnableToAuthorizeJoin(
            "Joining user did not pass restricted room's rules."
        )));
    }

    Ok(())
}
