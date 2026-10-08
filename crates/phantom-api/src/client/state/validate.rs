use futures::{FutureExt, TryFutureExt};
use phantom_core::{Err, Result, bool::BoolExt, err, is_false};
use phantom_service::Services;
use ruma::{
    OwnedRoomAliasId, RoomId, UserId,
    events::{
        AnyStateEventContent, StateEventType,
        room::{
            canonical_alias::RoomCanonicalAliasEventContent,
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            member::{MembershipState, RoomMemberEventContent},
            server_acl::RoomServerAclEventContent,
        },
    },
    serde::Raw,
};

pub(super) async fn allowed_to_send_state_event(
    services: &Services,
    room_id: &RoomId,
    event_type: &StateEventType,
    state_key: &str,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    match event_type {
        StateEventType::RoomCreate => Err!(Request(BadJson(debug_warn!(
            message =
                format_args!("You cannot update m.room.create after a room has been created."),
            ?room_id
        )))),
        StateEventType::RoomServerAcl => validate_server_acl(services, room_id, json),
        StateEventType::RoomEncryption => validate_encryption(services),
        StateEventType::RoomJoinRules => validate_join_rules(services, room_id, json).await,
        StateEventType::RoomHistoryVisibility => {
            validate_history_visibility(services, room_id, json).await
        }
        StateEventType::RoomCanonicalAlias => {
            validate_canonical_alias(services, room_id, json).await
        }
        StateEventType::RoomMember => validate_member(services, room_id, state_key, json).await,
        _ => Ok(()),
    }
}

fn validate_encryption(services: &Services) -> Result {
    services
        .config
        .client
        .allow_encryption
        .then_some(())
        .ok_or_else(|| {
            err!(Request(Forbidden(
                "Encryption is disabled on this homeserver."
            )))
        })
}

fn validate_server_acl(
    services: &Services,
    room_id: &RoomId,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    let acl_content = json
        .deserialize_as_unchecked::<RoomServerAclEventContent>()
        .map_err(|e| {
            err!(Request(BadJson(debug_warn!(
                "Room server ACL event is invalid: {e}"
            ))))
        })?;

    let allow_contains = |server: &str| acl_content.allow.iter().any(|allow| allow == server);
    let deny_contains = |server: &str| acl_content.deny.iter().any(|deny| deny == server);

    if acl_content.allow.is_empty() {
        return Err!(Request(BadJson(debug_warn!(
            message = format_args!(
                "Sending an ACL event with an empty allow key will permanently brick the room for \
             non-phantom servers as this equates to no servers being allowed to participate in this \
             room."
            ),
            ?room_id
        ))));
    }

    if deny_contains("*") && allow_contains("*") {
        return Err!(Request(BadJson(debug_warn!(
            message = format_args!(
                "Sending an ACL event with a deny and allow key value of \"*\" will permanently \
             brick the room for non-phantom servers as this equates to no servers being allowed to \
             participate in this room."
            ),
            ?room_id
        ))));
    }

    let server_name = services.server_state.server_name();
    let self_allowed = acl_content.is_allowed(server_name) || allow_contains(server_name.as_str());

    if deny_contains("*") && !self_allowed {
        return Err!(Request(BadJson(debug_warn!(
            message = format_args!(
                "Sending an ACL event with a deny key value of \"*\" and without your own server \
             name in the allow key will result in you being unable to participate in this room."
            ),
            ?room_id
        ))));
    }

    if !allow_contains("*") && !self_allowed {
        return Err!(Request(BadJson(debug_warn!(
            message = format_args!(
                "Sending an ACL event for an allow key without \"*\" and without your own server \
             name in the allow key will result in you being unable to participate in this room."
            ),
            ?room_id
        ))));
    }

    Ok(())
}

async fn validate_join_rules(
    services: &Services,
    room_id: &RoomId,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    let Ok(admin_room_id) = services.admin.get_admin_room().await else {
        return Ok(());
    };

    if admin_room_id != room_id {
        return Ok(());
    }

    let join_rule = json
        .deserialize_as_unchecked::<RoomJoinRulesEventContent>()
        .map_err(|e| {
            err!(Request(BadJson(debug_warn!(
                "Room join rules event is invalid: {e}"
            ))))
        })?;

    if join_rule.join_rule == JoinRule::Public {
        return Err!(Request(Forbidden(
            "Admin room is a sensitive room, it cannot be made public"
        )));
    }

    Ok(())
}

async fn validate_history_visibility(
    services: &Services,
    room_id: &RoomId,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    let Ok(admin_room_id) = services.admin.get_admin_room().await else {
        return Ok(());
    };

    let visibility_content = json
        .deserialize_as_unchecked::<RoomHistoryVisibilityEventContent>()
        .map_err(|e| {
            err!(Request(BadJson(debug_warn!(
                "Room history visibility event is invalid: {e}"
            ))))
        })?;

    if admin_room_id == room_id
        && visibility_content.history_visibility == HistoryVisibility::WorldReadable
    {
        return Err!(Request(Forbidden(
            "Admin room is a sensitive room, it cannot be made world readable (public room \
             history)."
        )));
    }

    Ok(())
}

async fn validate_canonical_alias(
    services: &Services,
    room_id: &RoomId,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    let canonical_alias_content = json
        .deserialize_as_unchecked::<RoomCanonicalAliasEventContent>()
        .map_err(|e| {
            err!(Request(InvalidParam(debug_warn!(
                "Room canonical alias event is invalid: {e}"
            ))))
        })?;

    let current_aliases: Vec<OwnedRoomAliasId> = services
        .rooms
        .state_accessor
        .room_state_get_content::<RoomCanonicalAliasEventContent>(
            room_id,
            &StateEventType::RoomCanonicalAlias,
            "",
        )
        .await
        .ok()
        .map(|content| {
            content
                .alias
                .into_iter()
                .chain(content.alt_aliases)
                .collect()
        })
        .unwrap_or_default();

    let new_aliases = canonical_alias_content
        .alias
        .iter()
        .chain(&canonical_alias_content.alt_aliases)
        .filter(|alias| !current_aliases.contains(alias));

    for alias in new_aliases {
        let (alias_room_id, _servers) = services
            .rooms
            .alias
            .resolve_alias(alias)
            .await
            .map_err(|e| err!(Request(BadAlias("Failed resolving alias \"{alias}\": {e}"))))?;

        if alias_room_id != room_id {
            return Err!(Request(BadAlias(
                "Room alias {alias} does not belong to room {room_id}"
            )));
        }
    }

    Ok(())
}

async fn validate_member(
    services: &Services,
    room_id: &RoomId,
    state_key: &str,
    json: &Raw<AnyStateEventContent>,
) -> Result {
    let membership_content = json
        .deserialize_as_unchecked::<RoomMemberEventContent>()
        .map_err(|e| {
            err!(Request(BadJson(
                "Membership content must have a valid JSON body with at least a valid \
                 membership state: {e}"
            )))
        })?;

    let Ok(target_user) = UserId::parse(state_key) else {
        return Err!(Request(BadJson(
            "Membership event has invalid or non-existent state key"
        )));
    };

    let Some(authorising_user) = membership_content.join_authorized_via_users_server else {
        return Ok(());
    };

    if membership_content.membership != MembershipState::Join {
        return Err!(Request(BadJson(
            "join_authorised_via_users_server is only for member joins"
        )));
    }

    // Already joined or invited: no restricted-join authorisation needed.
    if services
        .rooms
        .state_cache
        .user_membership(&target_user, room_id)
        .await
        .is_some_and(|m| matches!(m, MembershipState::Join | MembershipState::Invite))
    {
        return Ok(());
    }

    if !services.server_state.user_is_local(&authorising_user) {
        return Err!(Request(InvalidParam(
            "Authorising user {authorising_user} does not belong to this homeserver"
        )));
    }

    services
        .rooms
        .state_cache
        .is_joined(&authorising_user, room_id)
        .map(is_false!())
        .map(BoolExt::into_result)
        .map_err(|()| {
            err!(Request(InvalidParam(
                "Authorising user {authorising_user} is not in the room. They cannot authorise \
                 the join."
            )))
        })
        .await
}
