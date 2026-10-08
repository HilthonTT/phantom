mod initial_state;
mod invites;
mod power_levels;
mod push_rules;

use axum::extract::State;
use futures::FutureExt;
use phantom_core::{
    Err, Result, debug_info, err, info,
    matrix::{RoomVersion, StateKey, pdu::PduBuilder},
    warn,
};
use phantom_service::{Services, ops::appservice::RegistrationInfo, rooms::state::RoomMutexGuard};
use ruma::{
    CanonicalJsonObject, OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, RoomVersionId, UserId,
    api::client::room::{
        self,
        create_room::{self, v3::RoomPreset},
    },
    events::{
        TimelineEventType,
        room::{
            canonical_alias::RoomCanonicalAliasEventContent,
            create::RoomCreateEventContent,
            member::{MembershipState, RoomMemberEventContent},
        },
    },
    room_version_rules::RoomVersionRules,
};
use serde_json::{json, value::to_raw_value};

use crate::{client::utils::invite_check, router::Ruma};

pub(super) use self::push_rules::copy_room_push_rule;
use self::{
    initial_state::{apply_initial_state_pdus, apply_name_and_topic_pdus, apply_preset_state_pdus},
    invites::process_invites,
    power_levels::apply_power_levels_pdu,
    push_rules::copy_creator_predecessor_push_rule,
};

pub(crate) async fn create_room_route(
    State(services): State<crate::router::State>,
    body: Ruma<create_room::v3::Request>,
) -> Result<create_room::v3::Response> {
    can_create_room_check(&services, &body).await?;
    can_publish_directory_check(&services, &body).await?;

    // Figure out preset. We need it for preset specific events
    let preset = body.preset.clone().unwrap_or(match &body.visibility {
        room::Visibility::Public => RoomPreset::PublicChat,
        _ => RoomPreset::PrivateChat, // Room visibility should not be custom
    });

    // Determine room version
    let room_version = match &body.room_version {
        Some(version) if !RoomVersion::is_supported(version) => {
            return Err!(Request(UnsupportedRoomVersion(
                "This server does not support room version {version:?}"
            )));
        }
        Some(version) => version,
        None => &services.config.client.default_room_version,
    };

    let version_rules = room_version.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "This server does not support room version {room_version:?}"
        )))
    })?;

    let sender_user = body.sender_user();

    // Error on existing alias before committing to creation.
    let alias = match &body.room_alias_name {
        Some(alias) => {
            Some(room_alias_check(&services, alias, body.appservice_info.as_ref()).await?)
        }
        None => None,
    };

    // 1. Create the create event. Every supported room version still names its
    // room with a server-generated ID rather than the create event's hash.
    let (room_id, state_lock) =
        create_create_event_legacy(&services, &body, room_version, &version_rules).await?;

    // 2. Let the room creator join
    apply_creator_join_pdu(&services, &body, sender_user, &room_id, &state_lock)
        .boxed()
        .await?;

    // 3. Power levels
    apply_power_levels_pdu(
        &services,
        &body,
        &preset,
        &version_rules,
        sender_user,
        &room_id,
        &state_lock,
    )
    .boxed()
    .await?;

    // 4. Canonical room alias
    if let Some(room_alias_id) = &alias {
        apply_canonical_alias_pdu(&services, room_alias_id, sender_user, &room_id, &state_lock)
            .boxed()
            .await?;
    }

    // 5. Events set by preset
    let initial_state = apply_preset_state_pdus(
        &services,
        &body,
        &preset,
        sender_user,
        &room_id,
        &state_lock,
    )
    .boxed()
    .await?;

    // 6. Events listed in initial_state
    apply_initial_state_pdus(
        &services,
        initial_state,
        &preset,
        sender_user,
        &room_id,
        &state_lock,
    )
    .boxed()
    .await?;

    // 7. Events implied by name and topic
    apply_name_and_topic_pdus(&services, &body, sender_user, &room_id, &state_lock)
        .boxed()
        .await?;

    drop(state_lock);

    // if inviting anyone with room creation and invite check passes
    if (!body.invite.is_empty() || !body.invite_3pid.is_empty())
        && invite_check(&services, sender_user, &room_id).await.is_ok()
    {
        process_invites(&services, &body, sender_user, &room_id)
            .boxed()
            .await;
    }

    finalize_alias_and_directory(&services, &body, alias.as_deref(), sender_user, &room_id).await?;

    copy_creator_predecessor_push_rule(
        &services,
        body.creation_content.as_ref(),
        sender_user,
        &room_id,
    )
    .await;

    info!("{sender_user} created a room with room ID {room_id}");

    Ok(create_room::v3::Response::new(room_id))
}

async fn apply_creator_join_pdu(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let mut content = RoomMemberEventContent::new(MembershipState::Join);
    content.is_direct = body.is_direct.then_some(true);
    services
        .profile
        .fill_profile_data(sender_user, &mut content)
        .await;

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(sender_user.to_string(), &content),
            sender_user,
            room_id,
            state_lock,
        )
        .await
        .map(|_| ())
}

async fn apply_canonical_alias_pdu(
    services: &Services,
    room_alias_id: &RoomAliasId,
    sender_user: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result {
    let mut canonical_alias = RoomCanonicalAliasEventContent::new();
    canonical_alias.alias = Some(room_alias_id.to_owned());

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &canonical_alias),
            sender_user,
            room_id,
            state_lock,
        )
        .await
        .map(|_| ())
}

async fn finalize_alias_and_directory(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    alias: Option<&RoomAliasId>,
    sender_user: &UserId,
    room_id: &RoomId,
) -> Result {
    if let Some(alias) = alias {
        services
            .rooms
            .alias
            .set_alias_by(alias, room_id, sender_user)?;
    }

    if body.visibility == room::Visibility::Public {
        services.rooms.directory.set_public(room_id)?;

        info!(
            "{sender_user} made {0} public to the room directory",
            room_id
        );
    }

    Ok(())
}

async fn create_create_event_legacy(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
    room_version: &RoomVersionId,
    version_rules: &RoomVersionRules,
) -> Result<(OwnedRoomId, RoomMutexGuard)> {
    let room_id = new_room_id(services).await?;

    let state_lock = services.rooms.state.mutex.lock(&*room_id).await;

    let _short_id = services
        .rooms
        .short
        .get_or_create_shortroomid(&room_id)
        .await;

    let create_content = match &body.creation_content {
        Some(content) => {
            let mut content = content
                .deserialize_as_unchecked::<CanonicalJsonObject>()
                .map_err(|e| {
                    err!(Request(BadJson(error!(
                        "Failed to deserialise content as canonical JSON: {e}"
                    ))))
                })?;

            if !version_rules.authorization.use_room_create_sender {
                content.insert(
                    "creator".into(),
                    json!(body.sender_user()).try_into().map_err(|e| {
                        err!(Request(BadJson(debug_error!(
                            "Invalid creation content: {e}"
                        ))))
                    })?,
                );
            }

            if !services.config.client.federate_created_rooms
                && (!services.config.federation.allow_federation
                    || !content.contains_key("m.federate"))
            {
                content.insert("m.federate".into(), json!(false).try_into()?);
            }

            content.insert(
                "room_version".into(),
                json!(room_version.as_str())
                    .try_into()
                    .map_err(|e| err!(Request(BadJson("Invalid creation content: {e}"))))?,
            );

            content
        }
        None => {
            let content = if !version_rules.authorization.use_room_create_sender {
                RoomCreateEventContent::new_v1(body.sender_user().to_owned())
            } else {
                RoomCreateEventContent::new_v11()
            };

            let mut content =
                serde_json::from_str::<CanonicalJsonObject>(to_raw_value(&content)?.get())?;

            if !services.config.client.federate_created_rooms {
                content.insert("m.federate".into(), json!(false).try_into()?);
            }

            content.insert(
                "room_version".into(),
                json!(room_version.as_str()).try_into()?,
            );
            content
        }
    };

    // 1. The room create event
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomCreate,
                content: to_raw_value(&create_content)?,
                state_key: Some(StateKey::new()),
                ..Default::default()
            },
            body.sender_user(),
            &room_id,
            &state_lock,
        )
        .boxed()
        .await?;

    Ok((room_id, state_lock))
}

/// if a room is being created with a room alias, run our checks
async fn room_alias_check(
    services: &Services,
    room_alias_name: &str,
    appservice_info: Option<&RegistrationInfo>,
) -> Result<OwnedRoomAliasId> {
    // Basic checks on the room alias validity
    if room_alias_name.contains(':') {
        return Err!(Request(InvalidParam(
            "Room alias contained `:` which is not allowed. Please note that this expects a \
             localpart, not the full room alias."
        )));
    } else if room_alias_name.contains(char::is_whitespace) {
        return Err!(Request(InvalidParam(
            "Room alias contained spaces which is not a valid room alias."
        )));
    }

    // check if room alias is forbidden
    if services
        .config
        .rooms
        .forbidden_alias_names
        .is_match(room_alias_name)
    {
        return Err!(Request(Unknown("Room alias name is forbidden.")));
    }

    let server_name = services.server_state.server_name();
    let full_room_alias =
        RoomAliasId::parse(format!("#{room_alias_name}:{server_name}")).map_err(|e| {
            err!(Request(InvalidParam(debug_error!(
                message = format_args!("Failed to parse room alias."),
                ?e,
                ?room_alias_name
            ))))
        })?;

    if services
        .rooms
        .alias
        .resolve_local_alias(&full_room_alias)
        .await
        .is_ok()
    {
        return Err!(Request(RoomInUse("Room alias already exists.")));
    }

    if let Some(info) = appservice_info {
        if !info.aliases.is_match(full_room_alias.as_str()) {
            return Err!(Request(Exclusive("Room alias is not in namespace.")));
        }
    } else if services
        .appservice
        .is_exclusive_alias(&full_room_alias)
        .await
    {
        return Err!(Request(Exclusive("Room alias reserved by appservice.")));
    }

    debug_info!("Full room alias: {full_room_alias}");

    Ok(full_room_alias)
}

/// Generates a fresh room ID on this server that no known room uses yet.
pub(super) async fn new_room_id(services: &Services) -> Result<OwnedRoomId> {
    let server_name = services.server_state.server_name();

    loop {
        let localpart = phantom_core::rand::string(18);
        let room_id = RoomId::parse(format!("!{localpart}:{server_name}"))?;

        if services
            .rooms
            .short
            .get_shortroomid(&room_id)
            .await
            .is_err()
        {
            return Ok(room_id);
        }
    }
}

async fn can_publish_directory_check(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
) -> Result {
    if !services.server.config.client.lockdown_public_room_directory
        || body.appservice_info.is_some()
        || body.visibility != room::Visibility::Public
        || services.admin.user_is_admin(body.sender_user()).await
    {
        return Ok(());
    }

    let msg = format!(
        "Non-admin user {} tried to publish new to the directory while \
         lockdown_public_room_directory is enabled",
        body.sender_user(),
    );

    warn!("{msg}");

    Err!(Request(Forbidden(
        "Publishing rooms to the room directory is not allowed"
    )))
}

async fn can_create_room_check(
    services: &Services,
    body: &Ruma<create_room::v3::Request>,
) -> Result {
    if !services.config.rooms.allow_room_creation
        && body.appservice_info.is_none()
        && !services.admin.user_is_admin(body.sender_user()).await
    {
        return Err!(Request(Forbidden("Room creation has been disabled.")));
    }

    Ok(())
}
