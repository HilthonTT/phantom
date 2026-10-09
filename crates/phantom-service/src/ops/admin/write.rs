//! The admin room's write paths: creating it on a fresh database, granting
//! admin by joining a user to it, and posting the server user's notices.

use std::collections::BTreeMap;

use phantom_core::{Err, Result, debug_info, err, error, implement, matrix::PduBuilder, rand};
use ruma::{
    OwnedRoomId, RoomId, UserId,
    events::{
        RoomAccountDataEventType, StateEventContent, StateEventType,
        room::{
            canonical_alias::RoomCanonicalAliasEventContent,
            create::RoomCreateEventContent,
            guest_access::{GuestAccess, RoomGuestAccessEventContent},
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            member::{MembershipState, RoomMemberEventContent},
            message::RoomMessageEventContent,
            name::RoomNameEventContent,
            power_levels::RoomPowerLevelsEventContent,
            topic::RoomTopicEventContent,
        },
        tag::{TagEvent, TagEventContent, TagInfo},
    },
};

use crate::Services;

/// A state event with the empty state key, which every room-wide one uses.
fn state<T: StateEventContent>(content: &T) -> PduBuilder {
    PduBuilder::state(String::new(), content)
}

/// The server user's power level in the admin room: above any admin's, so no
/// admin can demote it.
const SERVER_USER_POWER: u32 = 69420;

const ADMIN_POWER: u32 = 100;

/// Posts a plain-text notice from the server user into the admin room.
#[implement(super::Service)]
pub async fn send_notice(&self, body: &str) -> Result {
    let content = RoomMessageEventContent::notice_plain(body);
    let server_user = &self.services.server_state.server_user;
    let room_id = self.get_admin_room().await?;

    let state_lock = self.services.state.mutex.lock(&*room_id).await;

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::timeline(&content),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    Ok(())
}

/// Makes a local user an admin: the server user invites them into the admin
/// room, they are joined, and their power level is raised.
///
/// A remote user is only invited; they become admin once they accept.
#[implement(super::Service)]
pub async fn make_user_admin(&self, user_id: &UserId) -> Result {
    let room_id = self.get_admin_room().await?;
    let state_lock = self.services.state.mutex.lock(&*room_id).await;

    if self.services.state_cache.is_joined(user_id, &room_id).await {
        return Err!(Request(InvalidParam("{user_id} is already an admin.")));
    }

    let server_user = &self.services.server_state.server_user;
    let member =
        |state| PduBuilder::state(user_id.to_string(), &RoomMemberEventContent::new(state));

    if !self
        .services
        .state_cache
        .is_invited(user_id, &room_id)
        .await
    {
        self.services
            .timeline
            .build_and_append_pdu(
                member(MembershipState::Invite),
                server_user,
                &room_id,
                &state_lock,
            )
            .await?;
    }

    if !self.services.server_state.user_is_local(user_id) {
        debug_info!(%user_id, %room_id, "Invited remote user to the admin room");
        return Ok(());
    }

    self.services
        .timeline
        .build_and_append_pdu(
            member(MembershipState::Join),
            user_id,
            &room_id,
            &state_lock,
        )
        .await?;

    let mut power_levels: RoomPowerLevelsEventContent = self
        .services
        .state_accessor
        .room_state_get_content(&room_id, &StateEventType::RoomPowerLevels, "")
        .await
        .map_err(|e| err!(Database("Admin room has no power levels: {e}")))?;

    power_levels
        .users
        .insert(server_user.clone(), SERVER_USER_POWER.into());
    power_levels
        .users
        .insert(user_id.to_owned(), ADMIN_POWER.into());

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &power_levels),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    let config = &self.services.server.config.admin;
    if !config.admin_room_tag.is_empty()
        && let Err(e) = self
            .set_room_tag(&room_id, user_id, &config.admin_room_tag)
            .await
    {
        error!(%room_id, %user_id, tag = %config.admin_room_tag, "Failed to tag the admin room: {e}");
    }

    if config.admin_room_notices {
        let welcome = RoomMessageEventContent::notice_plain(format!(
            "Welcome, {user_id}. You are now an admin of {}. Sign in to the phantom console \
             with this account to manage the server.",
            self.services.server.name
        ));

        self.services
            .timeline
            .build_and_append_pdu(
                PduBuilder::timeline(&welcome),
                server_user,
                &room_id,
                &state_lock,
            )
            .await?;
    }

    debug_info!(%user_id, %room_id, "Granted admin");
    Ok(())
}

/// Takes admin away: the server user removes the user from the admin room,
/// or withdraws a pending invite, and drops their power level there.
#[implement(super::Service)]
pub async fn revoke_admin(&self, user_id: &UserId) -> Result {
    let server_user = &self.services.server_state.server_user;
    if user_id == server_user {
        return Err!(Request(Forbidden(
            "The server user's admin cannot be revoked."
        )));
    }

    let room_id = self.get_admin_room().await?;
    let state_lock = self.services.state.mutex.lock(&*room_id).await;

    let state_cache = &self.services.state_cache;
    if !state_cache.is_joined(user_id, &room_id).await
        && !state_cache.is_invited(user_id, &room_id).await
    {
        return Err!(Request(InvalidParam("{user_id} is not an admin.")));
    }

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(
                user_id.to_string(),
                &RoomMemberEventContent::new(MembershipState::Leave),
            ),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    let mut power_levels: RoomPowerLevelsEventContent = self
        .services
        .state_accessor
        .room_state_get_content(&room_id, &StateEventType::RoomPowerLevels, "")
        .await
        .map_err(|e| err!(Database("Admin room has no power levels: {e}")))?;

    if power_levels.users.remove(user_id).is_some() {
        self.services
            .timeline
            .build_and_append_pdu(state(&power_levels), server_user, &room_id, &state_lock)
            .await?;
    }

    debug_info!(%user_id, %room_id, "Revoked admin");
    Ok(())
}

#[implement(super::Service)]
async fn set_room_tag(&self, room_id: &RoomId, user_id: &UserId, tag: &str) -> Result {
    let mut event: TagEvent = self
        .services
        .account_data
        .get_room(room_id, user_id, RoomAccountDataEventType::Tag)
        .await
        .unwrap_or_else(|_| TagEvent::new(TagEventContent::new(BTreeMap::new())));

    event
        .content
        .tags
        .insert(tag.to_owned().into(), TagInfo::new());

    self.services
        .account_data
        .update(
            Some(room_id),
            user_id,
            RoomAccountDataEventType::Tag,
            &serde_json::to_value(event)?,
        )
        .await
}

/// Creates the server user and the admin room it alone sits in, reached by
/// `#admins:<server_name>`. Run once, on a fresh database.
pub(crate) async fn create_admin_room(services: &Services) -> Result {
    let server_name = services.server_state.server_name();
    let server_user = &services.server_state.server_user;
    let config = &services.server.config;

    let room_version = &config.client.default_room_version;
    let rules = room_version.rules().ok_or_else(|| {
        err!(Config(
            "default_room_version",
            "{room_version} is not supported"
        ))
    })?;

    let room_id = OwnedRoomId::try_from(format!("!{}:{server_name}", rand::string(18)))?;
    services
        .rooms
        .short
        .get_or_create_shortroomid(&room_id)
        .await;

    let state_lock = services.rooms.state.mutex.lock(&*room_id).await;

    services.users.create(server_user, None)?;

    let append = async |builder: PduBuilder, sender: &UserId| {
        services
            .rooms
            .timeline
            .build_and_append_pdu(builder, sender, &room_id, &state_lock)
            .await
            .map(|_| ())
    };

    let mut create = if rules.authorization.use_room_create_sender {
        RoomCreateEventContent::new_v11()
    } else {
        RoomCreateEventContent::new_v1(server_user.clone())
    };
    create.room_version = room_version.clone();
    append(state(&create), server_user).await?;

    append(
        PduBuilder::state(
            server_user.to_string(),
            &RoomMemberEventContent::new(MembershipState::Join),
        ),
        server_user,
    )
    .await?;

    let mut power_levels = RoomPowerLevelsEventContent::new(&rules.authorization);
    power_levels
        .users
        .insert(server_user.clone(), SERVER_USER_POWER.into());
    append(state(&power_levels), server_user).await?;

    append(
        state(&RoomJoinRulesEventContent::new(JoinRule::Invite)),
        server_user,
    )
    .await?;
    append(
        state(&RoomHistoryVisibilityEventContent::new(
            HistoryVisibility::Shared,
        )),
        server_user,
    )
    .await?;
    append(
        state(&RoomGuestAccessEventContent::new(GuestAccess::Forbidden)),
        server_user,
    )
    .await?;

    append(
        state(&RoomNameEventContent::new(format!(
            "{server_name} Admin Room"
        ))),
        server_user,
    )
    .await?;
    append(
        state(&RoomTopicEventContent::new(format!(
            "Manage {server_name} | Admins in this room can use the phantom console"
        ))),
        server_user,
    )
    .await?;

    let alias = &services.server_state.admin_alias;
    let mut canonical_alias = RoomCanonicalAliasEventContent::new();
    canonical_alias.alias = Some(alias.clone());
    append(state(&canonical_alias), server_user).await?;

    services.rooms.alias.set_alias(alias, &room_id)?;

    debug_info!(%room_id, %alias, "Created the admin room");
    Ok(())
}
