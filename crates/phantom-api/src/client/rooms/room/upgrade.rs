use std::cmp::max;

use axum::extract::State;
use futures::{FutureExt, StreamExt, TryFutureExt, TryStreamExt};
use phantom_core::{
    Err, Result, debug_info, err, error, implement, info,
    matrix::{PduEvent, RoomVersion, StateKey, pdu::PduBuilder},
    stream::{IterStream, ReadyExt, WidebandExt},
};
use phantom_service::{Services, rooms::state::RoomMutexGuard};
use ruma::{
    CanonicalJsonObject, OwnedEventId, OwnedRoomId, RoomId, RoomVersionId, UserId,
    api::client::room::upgrade_room::v3,
    events::{
        StateEventType, TimelineEventType,
        room::{
            create::{PreviousRoom, RoomCreateEventContent},
            member::{MembershipState, RoomMemberEventContent},
            power_levels::RoomPowerLevelsEventContent,
            tombstone::RoomTombstoneEventContent,
        },
    },
    int,
    room::RoomType,
    room_version_rules::RoomVersionRules,
};
use serde_json::{json, value::to_raw_value};

use super::create::{copy_room_push_rule, new_room_id};
use crate::router::Ruma;

//TODO: Upgrade Ruma
const RECOMMENDED_TRANSFERABLE_STATE_EVENT_TYPES: &[StateEventType; 9] = &[
    StateEventType::RoomServerAcl,
    StateEventType::RoomEncryption,
    StateEventType::RoomName,
    StateEventType::RoomAvatar,
    StateEventType::RoomTopic,
    StateEventType::RoomGuestAccess,
    StateEventType::RoomHistoryVisibility,
    StateEventType::RoomJoinRules,
    StateEventType::RoomPowerLevels,
];

struct RoomUpgradeContext<'a> {
    services: &'a Services,
    sender_user: &'a UserId,
    creator: &'a UserId,
    old_room_id: &'a RoomId,
    old_state_lock: &'a RoomMutexGuard,
    new_room_id: &'a RoomId,
    new_state_lock: &'a RoomMutexGuard,
}

/// # `POST /_matrix/client/r0/rooms/{roomId}/upgrade`
///
/// Upgrades the room.
///
/// - Creates a replacement room
/// - Sends a tombstone event into the current room
/// - Sender user joins the room
/// - Transfers some state events
/// - Moves local aliases
/// - Modifies old room power levels to prevent users from speaking
#[tracing::instrument(level = "debug", skip_all)]
pub(crate) async fn upgrade_room_route(
    State(services): State<crate::router::State>,
    body: Ruma<v3::Request>,
) -> Result<v3::Response> {
    let sender_user = body.sender_user();
    let new_version = &body.new_version;

    if !RoomVersion::is_supported(new_version) {
        return Err!(Request(UnsupportedRoomVersion(
            "This server does not support that room version."
        )));
    }

    let version_rules = new_version.rules().ok_or_else(|| {
        err!(Request(UnsupportedRoomVersion(
            "This server does not support that room version."
        )))
    })?;

    let old_room_id = &body.room_id;
    let old_state_lock = services.rooms.state.mutex.lock(&**old_room_id).await;

    if !services
        .rooms
        .state_accessor
        .room_power_levels(old_room_id)
        .await
        .user_can_send_state(sender_user, StateEventType::RoomTombstone)
    {
        return Err!(Request(Forbidden(
            "You are not permitted to upgrade the room."
        )));
    }

    let latest_event_id = services
        .rooms
        .timeline
        .latest_pdu_in_room(old_room_id)
        .await
        .ok()
        .map(|pdu| pdu.event_id);

    let mut predecessor = PreviousRoom::new(old_room_id.to_owned());
    // Every room version this server supports still carries the predecessor's
    // last event ID.
    #[expect(deprecated)]
    {
        predecessor.event_id.clone_from(&latest_event_id);
    }

    debug_info!(
        %sender_user,
        %old_room_id,
        last_event = ?latest_event_id,
        ?new_version,
        "Attempting upgrade of room..."
    );

    let creator = if services.admin.is_admin_room(&body.room_id).await {
        &services.server_state.server_user
    } else {
        sender_user
    };

    let (replacement_room, state_lock) = upgrade_room_create_legacy(
        &services,
        creator,
        old_room_id,
        new_version,
        &version_rules,
        predecessor,
    )
    .await
    .inspect_err(|e| error!(%old_room_id, "Upgrade m.room.create event failed: {e}"))?;

    let context = RoomUpgradeContext {
        services: &services,
        sender_user,
        creator,
        old_room_id,
        old_state_lock: &old_state_lock,
        new_room_id: &replacement_room,
        new_state_lock: &state_lock,
    };

    if let Err(e) = context.transfer_room().await {
        error!(?e, %old_room_id, %replacement_room, "Room upgrade failed. Cleaning up incomplete room...");

        if let Err(e) = services
            .rooms
            .delete
            .delete_room(&replacement_room, false, &state_lock)
            .await
        {
            error!("Additional errors while deleting incomplete room: {e}");
        }

        return Err(e);
    }

    info!(
        old_room_id = %context.old_room_id,
        new_room_id = %context.new_room_id,
        upgraded_by = %sender_user,
        "Room upgraded",
    );

    Ok(v3::Response::new(replacement_room))
}

#[tracing::instrument(level = "info", skip_all)]
async fn upgrade_room_create_legacy(
    services: &Services,
    sender_user: &UserId,
    old_room_id: &RoomId,
    new_version: &RoomVersionId,
    version_rules: &RoomVersionRules,
    predecessor: PreviousRoom,
) -> Result<(OwnedRoomId, RoomMutexGuard)> {
    // Create a replacement room
    let new_room_id = new_room_id(services).await?;
    let state_lock = services.rooms.state.mutex.lock(&*new_room_id).await;
    let _short_id = services
        .rooms
        .short
        .get_or_create_shortroomid(&new_room_id)
        .await;

    // Get the old room creation event
    let mut content: CanonicalJsonObject = services
        .rooms
        .state_accessor
        .room_state_get_content(old_room_id, &StateEventType::RoomCreate, "")
        .await
        .map_err(|_| err!(Database("Found room without m.room.create event.")))?;

    // Send a m.room.create event containing a predecessor field and the applicable
    // room_version. "creator" key no longer exists in V11+ rooms.
    if !version_rules.authorization.use_room_create_sender {
        content.insert("creator".into(), json!(&sender_user).try_into()?);
    } else {
        content.remove("creator");
    }

    content.insert("predecessor".into(), json!(predecessor).try_into()?);
    content.insert("room_version".into(), json!(new_version).try_into()?);

    // Validate creation event content
    let raw_content = to_raw_value(&content)?;
    if let Err(e) = serde_json::from_str::<CanonicalJsonObject>(raw_content.get()) {
        return Err!(Request(BadJson("Error forming creation event: {e}")));
    }

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder {
                event_type: TimelineEventType::RoomCreate,
                content: to_raw_value(&content)?,
                state_key: Some(StateKey::new()),
                ..Default::default()
            },
            sender_user,
            &new_room_id,
            &state_lock,
        )
        .await?;

    Ok((new_room_id, state_lock))
}

#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn transfer_room(&self) -> Result {
    self.move_creator().await?;

    self.move_state_events().await?;

    self.move_space_state().await?;

    self.move_sender_user().await?;

    self.move_push_rules().await?;

    self.move_local_aliases().await?;

    self.tombstone_old_room().await?;

    // After commitment to the tombstone above no more errors can propagate.
    self.lockdown_old_room()
        .await
        .inspect_err(
            |e| error!(old_room_id = %self.old_room_id, "Failed to lockdown old room: {e}"),
        )
        .ok();

    Ok(())
}

// Join the new room
#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_creator(&self) -> Result {
    self.move_member(self.creator).await?;

    Ok(())
}

#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_sender_user(&self) -> Result {
    if self.sender_user != self.creator {
        self.services
            .rooms
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(
                    self.sender_user.as_str(),
                    &RoomMemberEventContent::new(MembershipState::Invite),
                ),
                self.creator,
                self.new_room_id,
                self.new_state_lock,
            )
            .await?;

        self.move_member(self.sender_user).await?;
    }

    Ok(())
}

#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_push_rules(&self) -> Result {
    copy_room_push_rule(
        self.services,
        self.sender_user,
        self.old_room_id,
        self.new_room_id,
    )
    .await
}

#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_member(&self, user_id: &UserId) -> Result {
    let mut content: RoomMemberEventContent = self
        .services
        .rooms
        .state_accessor
        .room_state_get_content(
            self.old_room_id,
            &StateEventType::RoomMember,
            user_id.as_str(),
        )
        .inspect_err(|e| error!(%user_id, "Missing room member event: {e}"))
        .await?;

    content.membership = MembershipState::Join;
    content.join_authorized_via_users_server = None;

    self.services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(user_id.as_str(), &content),
            user_id,
            self.new_room_id,
            self.new_state_lock,
        )
        .await?;

    Ok(())
}

// Replicate transferable state events to the new room
#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_state_events(&self) -> Result {
    RECOMMENDED_TRANSFERABLE_STATE_EVENT_TYPES
        .iter()
        .rev()
        .stream()
        .wide_filter_map(|event_type| {
            self.services
                .rooms
                .state_accessor
                .room_state_get(self.old_room_id, event_type, "")
                .map(Result::ok)
        })
        .map(Ok)
        .try_for_each(async |event| {
            self.services
                .rooms
                .timeline
                .build_and_append_pdu(
                    self.rebuild_state_event(&event).await?,
                    self.creator,
                    self.new_room_id,
                    self.new_state_lock,
                )
                .inspect_err(|e| {
                    error!(event_id = %event.event_id, "Failed to transfer state on upgrade: {e}");
                })
                .map_ok(|_| ())
                .await
        })
        .await
}

// MSC4168: copy m.space.parent for any room, plus m.space.child when the
// old room is itself a space, into the upgraded room.
#[implement(RoomUpgradeContext, params = "<'_>")]
// try_for_each requires FnMut returning a nameable future; an async closure
// capturing self does not satisfy it.
#[expect(closure_returning_async_block)]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_space_state(&self) -> Result {
    let old_room_is_space = self
        .services
        .rooms
        .state_accessor
        .room_state_get_content::<RoomCreateEventContent>(
            self.old_room_id,
            &StateEventType::RoomCreate,
            "",
        )
        .await
        .ok()
        .and_then(|c| c.room_type)
        .is_some_and(|t| matches!(t, RoomType::Space));

    let event_types: &[StateEventType] = match old_room_is_space {
        true => &[StateEventType::SpaceParent, StateEventType::SpaceChild],
        false => &[StateEventType::SpaceParent],
    };

    let shortstatehash = self
        .services
        .rooms
        .state
        .get_room_shortstatehash(self.old_room_id)
        .await?;

    event_types
        .iter()
        .stream()
        .map(Ok)
        .try_for_each(|event_type| {
            self.services
                .rooms
                .state_accessor
                .state_keys(shortstatehash, event_type)
                .map(Ok)
                .try_for_each(move |state_key| async move {
                    let Ok(event) = self
                        .services
                        .rooms
                        .state_accessor
                        .room_state_get(self.old_room_id, event_type, &state_key)
                        .await
                    else {
                        return Ok(());
                    };

                    self.services
                        .rooms
                        .timeline
                        .build_and_append_pdu(
                            self.rebuild_state_event(&event).await?,
                            self.creator,
                            self.new_room_id,
                            self.new_state_lock,
                        )
                        .inspect_err(|e| {
                            error!(event_id = %event.event_id, "Failed to copy space state: {e}");
                        })
                        .await
                        .ok();

                    Ok(())
                })
        })
        .await
}

#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn rebuild_state_event(&self, event: &PduEvent) -> Result<PduBuilder> {
    let content = match event.kind {
        // MSC4168: rewrite `via` to the upgrading server's name on copied
        // space-graph state events, since the previous room's via list may
        // no longer cover the upgraded room.
        TimelineEventType::SpaceChild | TimelineEventType::SpaceParent => {
            let mut content = event.get_content_as_value();
            if let Some(obj) = content.as_object_mut() {
                obj.insert(
                    "via".to_owned(),
                    json!([self.sender_user.server_name().as_str()]),
                );
            }

            to_raw_value(&content)?
        }
        _ => event.content.clone(),
    };

    Ok(PduBuilder {
        content,
        event_type: event.kind.clone(),
        state_key: event.state_key.clone(),
        ..Default::default()
    })
}

// Moves any local aliases to the new room
#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn move_local_aliases(&self) -> Result {
    self.services
        .rooms
        .alias
        .local_aliases_for_room(self.old_room_id)
        .ready_for_each(|alias| {
            self.services
                .rooms
                .alias
                .set_alias_by(alias, self.new_room_id, self.creator)
                .inspect_err(|e| error!(%alias, "Failed to add alias: {e}"))
                .ok();
        })
        .map(Ok)
        .await
}

// Send a m.room.tombstone event to the old room to indicate that it is not
// intended to be used any further Fail if the sender does not have the required
// permissions.
#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn tombstone_old_room(&self) -> Result<OwnedEventId> {
    self.services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(
                StateKey::new(),
                &RoomTombstoneEventContent::new(
                    "This room has been upgraded.".to_owned(),
                    self.new_room_id.to_owned(),
                ),
            ),
            self.sender_user,
            self.old_room_id,
            self.old_state_lock,
        )
        .await
}

// Modify the power levels in the old room to prevent sending of events and
// inviting new users. Though a Result is returned, the callsite above treats it
// as infallible because the tombstone represents the commitment.
#[implement(RoomUpgradeContext, params = "<'_>")]
#[tracing::instrument(level = "debug", skip_all)]
async fn lockdown_old_room(&self) -> Result<OwnedEventId> {
    // Get the old room power levels
    let mut content: RoomPowerLevelsEventContent = self
        .services
        .rooms
        .state_accessor
        .room_state_get_content(self.old_room_id, &StateEventType::RoomPowerLevels, "")
        .await
        .map_err(|_| err!(Database("Found room without m.room.power_levels event.")))?;

    let old_users_default = content.users_default.checked_add(int!(1)).ok_or_else(|| {
        err!(Request(BadJson(
            "users_default power levels event content is not valid"
        )))
    })?;

    // Setting events_default and invite to the greater of 50 and users_default + 1
    let new_level = max(int!(50), old_users_default);
    content.events_default = new_level;
    content.invite = new_level;

    self.services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(StateKey::new(), &content),
            self.sender_user,
            self.old_room_id,
            self.old_state_lock,
        )
        .await
}
