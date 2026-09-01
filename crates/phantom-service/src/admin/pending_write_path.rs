//! The parts of the admin service that append events, waiting on the write
//! path they append through.
//!
//! This file is deliberately not a module of `admin`: nothing declares `mod
//! pending_write_path;`, so it is not compiled. It follows the same convention
//! as `rooms/timeline/pending_write_path.rs`, and for the same reason — this
//! is conduwuit's code kept close to as-pasted so it can be ported function by
//! function as its dependencies land, rather than rewritten from memory later.
//!
//! Every function here calls
//! [`rooms::timeline::build_and_append_pdu`](crate::rooms::timeline), which is
//! itself parked. What each one additionally waits on:
//!
//! - `send_text` / `send_message` / `respond_to_room` / `handle_response`:
//!   `rooms::state` (the per-room mutex), `rooms::timeline`
//! - `make_user_admin`: `rooms::state`, `rooms::state_cache`,
//!   `rooms::state_accessor`, `account_data`, and the `admin_room_tag` and
//!   `admin_room_notices` config options, which are not in `Config` yet
//!   because nothing compiled reads them
//! - `create_admin_room`: `rooms::alias` (`set_alias`), `rooms::short`,
//!   `rooms::state`, `rooms::timeline`, `users`
//!
//! Names to map when porting, beyond `conduwuit` → `phantom_core`:
//!
//! - `services.globals.server_user` / `.user_is_local()` / `.server_name()` →
//!   `services.server_state`, and `services.globals.admin_alias` →
//!   `server_state.admin_room_id()`, which resolves the alias itself
//! - `services.state`, `.state_cache`, `.state_accessor`, `.timeline` are
//!   `Dep`s this service does not hold yet; only `state_cache` is in
//!   [`super::Services`] today
//! - `RoomMessageEventContent::text_markdown` needs ruma's `markdown`
//!   feature, which this workspace does not enable; the welcome message is the
//!   only caller
//!
//! The imports are conduwuit's, left alone for the same reason the bodies are:
//!
//! ```ignore
//! use std::collections::BTreeMap;
//!
//! use conduwuit::{
//!     Err, Error, Result, debug_info, debug_warn, error, error::default_log, implement,
//!     matrix::pdu::PduBuilder,
//! };
//! use futures::FutureExt;
//! use ruma::{
//!     RoomId, RoomVersionId, UserId,
//!     events::{
//!         RoomAccountDataEventType, StateEventType,
//!         room::{
//!             canonical_alias::RoomCanonicalAliasEventContent,
//!             create::RoomCreateEventContent,
//!             guest_access::{GuestAccess, RoomGuestAccessEventContent},
//!             history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
//!             join_rules::{JoinRule, RoomJoinRulesEventContent},
//!             member::{MembershipState, RoomMemberEventContent},
//!             message::RoomMessageEventContent,
//!             name::RoomNameEventContent,
//!             power_levels::RoomPowerLevelsEventContent,
//!             preview_url::RoomPreviewUrlsEventContent,
//!             topic::RoomTopicEventContent,
//!         },
//!         tag::{TagEvent, TagEventContent, TagInfo},
//!     },
//! };
//!
//! use crate::rooms::state::RoomMutexGuard;
//! ```

/// Sends a markdown message to the admin room as the server user.
///
/// Not an `m.notice`: a notice does not notify, and the admin room is where
/// the server says things an operator is meant to see.
#[implement(super::Service)]
pub async fn send_text(&self, body: &str) {
    self.send_message(RoomMessageEventContent::text_markdown(body))
        .await
        .ok();
}

/// Sends a message to the admin room as the server user.
#[implement(super::Service)]
pub async fn send_message(&self, message_content: RoomMessageEventContent) -> Result {
    let user_id = &self.services.server_state.server_user;
    let room_id = self.get_admin_room().await?;

    self.respond_to_room(message_content, &room_id, user_id)
        .boxed()
        .await
}

/// Delivers a command's output as a reply to the event that asked for it.
///
/// This is what [`super::Service::handle_command`] logs in place of today.
#[implement(super::Service)]
async fn handle_response(&self, content: RoomMessageEventContent) -> Result {
    let Some(Relation::Reply { in_reply_to }) = content.relates_to.as_ref() else {
        return Ok(());
    };

    let Ok(pdu) = self.services.timeline.get_pdu(&in_reply_to.event_id).await else {
        error!(
            event_id = ?in_reply_to.event_id,
            "Missing admin command in_reply_to event"
        );
        return Ok(());
    };

    // In the admin room the server answers as itself; anywhere else the
    // command was escaped by an admin, and the echo is theirs.
    let response_sender = if self.is_admin_room(&pdu.room_id).await {
        &self.services.server_state.server_user
    } else {
        &pdu.sender
    };

    self.respond_to_room(content, &pdu.room_id, response_sender)
        .boxed()
        .await
}

#[implement(super::Service)]
async fn respond_to_room(
    &self,
    content: RoomMessageEventContent,
    room_id: &RoomId,
    user_id: &UserId,
) -> Result {
    assert!(self.user_is_admin(user_id).await, "sender is not admin");

    let state_lock = self.services.state.mutex.lock(room_id).await;

    if let Err(e) = self
        .services
        .timeline
        .build_and_append_pdu(PduBuilder::timeline(&content), user_id, room_id, &state_lock)
        .await
    {
        self.handle_response_error(e, room_id, user_id, &state_lock)
            .await
            .unwrap_or_else(default_log);
    }

    Ok(())
}

/// Reports that the output could not be delivered, in the room the output was
/// for. A command that ran but could not answer is worse than one that failed,
/// because the operator cannot tell the two apart.
#[implement(super::Service)]
async fn handle_response_error(
    &self,
    e: Error,
    room_id: &RoomId,
    user_id: &UserId,
    state_lock: &RoomMutexGuard,
) -> Result {
    error!("Failed to build and append admin room response PDU: \"{e}\"");
    let content = RoomMessageEventContent::text_plain(format!(
        "Failed to build and append admin room PDU: \"{e}\"\n\nThe original admin command may \
         have finished successfully, but we could not return the output."
    ));

    self.services
        .timeline
        .build_and_append_pdu(PduBuilder::timeline(&content), user_id, room_id, state_lock)
        .await?;

    Ok(())
}

/// Invites a user to the admin room, which is what granting admin is.
#[implement(super::Service)]
pub async fn make_user_admin(&self, user_id: &UserId) -> Result {
    let Ok(room_id) = self.get_admin_room().await else {
        debug_warn!("make_user_admin was called without an admin room being available or created");
        return Ok(());
    };

    let state_lock = self.services.state.mutex.lock(&room_id).await;

    if self.services.state_cache.is_joined(user_id, &room_id).await {
        return Err!(debug_warn!("User is already joined in the admin room"));
    }

    if self.services.state_cache.is_invited(user_id, &room_id).await {
        return Err!(debug_warn!("User is already pending an invitation to the admin room"));
    }

    // The server user is what grants the membership and the power level; it is
    // the only account that already has the power to.
    let server_user = &self.services.server_state.server_user;

    if self.services.server_state.user_is_local(user_id) {
        // A local user is joined outright. The invite is still sent, because
        // the join has to have something to accept.
        debug_info!("Inviting local user {user_id} to admin room {room_id}");
        self.services
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(
                    String::from(user_id),
                    &RoomMemberEventContent::new(MembershipState::Invite),
                ),
                server_user,
                &room_id,
                &state_lock,
            )
            .await?;

        debug_info!("Force joining local user {user_id} to admin room {room_id}");
        self.services
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(
                    String::from(user_id),
                    &RoomMemberEventContent::new(MembershipState::Join),
                ),
                user_id,
                &room_id,
                &state_lock,
            )
            .await?;
    } else {
        // A remote user has to accept for themselves.
        debug_info!("Inviting remote user {user_id} to admin room {room_id}");
        self.services
            .timeline
            .build_and_append_pdu(
                PduBuilder::state(
                    user_id.to_string(),
                    &RoomMemberEventContent::new(MembershipState::Invite),
                ),
                server_user,
                &room_id,
                &state_lock,
            )
            .await?;
    }

    let mut room_power_levels = self
        .services
        .state_accessor
        .room_state_get_content::<RoomPowerLevelsEventContent>(
            &room_id,
            &StateEventType::RoomPowerLevels,
            "",
        )
        .await
        .unwrap_or_default();

    room_power_levels.users.insert(server_user.into(), 69420.into());
    room_power_levels.users.insert(user_id.into(), 100.into());

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &room_power_levels),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // Tagging the room is a convenience for the client, so a failure is
    // reported and not fatal to the grant.
    let room_tag = self.services.server.config.admin_room_tag.as_str();
    if !room_tag.is_empty()
        && let Err(e) = self.set_room_tag(&room_id, user_id, room_tag).await
    {
        error!(?room_id, ?user_id, ?room_tag, "Failed to set tag for admin grant: {e}");
    }

    if self.services.server.config.admin_room_notices {
        let welcome_message = String::from(
            "## Thank you for trying out phantom!\n\nphantom is a Matrix homeserver derived \
             from conduwuit.\n\nFor a list of available commands, send the following message in \
             this room: `!admin --help`",
        );

        self.services
            .timeline
            .build_and_append_pdu(
                PduBuilder::timeline(&RoomMessageEventContent::text_markdown(welcome_message)),
                server_user,
                &room_id,
                &state_lock,
            )
            .await?;
    }

    Ok(())
}

/// Adds a tag to the new admin's copy of the admin room, so their client can
/// sort it away from their ordinary rooms.
#[implement(super::Service)]
async fn set_room_tag(&self, room_id: &RoomId, user_id: &UserId, tag: &str) -> Result {
    let mut event: TagEvent = self
        .services
        .account_data
        .get_room(room_id, user_id, RoomAccountDataEventType::Tag)
        .await
        .unwrap_or_else(|_| TagEvent {
            content: TagEventContent { tags: BTreeMap::new() },
        });

    event.content.tags.insert(tag.to_owned().into(), TagInfo::new());

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

/// Creates the admin room.
///
/// Taking the whole service graph rather than being a method: this runs once,
/// from startup, and touches more services than the admin service holds.
pub async fn create_admin_room(services: &Services) -> Result {
    let room_id = RoomId::new(services.server_state.server_name());
    let room_version = &services.server.config.default_room_version;

    let _short_id = services.rooms.short.get_or_create_shortroomid(&room_id).await;

    let state_lock = services.rooms.state.mutex.lock(&room_id).await;

    // The account the server posts as has to exist before it can post.
    let server_user = &services.server_state.server_user;
    services.users.create(server_user, None)?;

    let create_content = {
        use RoomVersionId::*;
        match room_version {
            V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 =>
                RoomCreateEventContent::new_v1(server_user.into()),
            _ => RoomCreateEventContent::new_v11(),
        }
    };

    // 1. The room create event
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomCreateEventContent {
                federate: true,
                predecessor: None,
                room_version: room_version.clone(),
                ..create_content
            }),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 2. Make the server user join
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(
                String::from(server_user),
                &RoomMemberEventContent::new(MembershipState::Join),
            ),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 3. Power levels
    let users = BTreeMap::from_iter([(server_user.into(), 69420.into())]);

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomPowerLevelsEventContent {
                users,
                ..Default::default()
            }),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 4.1 Join rules — invite only, since the invite is the grant
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomJoinRulesEventContent::new(JoinRule::Invite)),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 4.2 History visibility
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(
                String::new(),
                &RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared),
            ),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 4.3 Guest access
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(
                String::new(),
                &RoomGuestAccessEventContent::new(GuestAccess::Forbidden),
            ),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 5. Name and topic
    let room_name = format!("{} Admin Room", services.server.config.server_name);
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomNameEventContent::new(room_name)),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomTopicEventContent {
                topic: format!(
                    "Manage {} | Run commands prefixed with `!admin` | Run `!admin -h` for help",
                    services.server.config.server_name
                ),
            }),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    // 6. Room alias. This is what `server_state::admin_room_id` resolves, so
    // the room is not the admin room until this lands.
    let alias = &services.server_state.admin_alias;

    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomCanonicalAliasEventContent {
                alias: Some(alias.clone()),
                alt_aliases: Vec::new(),
            }),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    services.rooms.alias.set_alias(alias, &room_id, server_user)?;

    // 7. Disable URL previews, which would otherwise have the server fetch
    // whatever a command's output happens to link to.
    services
        .rooms
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(String::new(), &RoomPreviewUrlsEventContent { disabled: true }),
            server_user,
            &room_id,
            &state_lock,
        )
        .await?;

    Ok(())
}
