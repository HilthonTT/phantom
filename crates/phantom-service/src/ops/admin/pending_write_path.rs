#[implement(super::Service)]
pub async fn send_text(&self, body: &str) {
    self.send_message(RoomMessageEventContent::text_markdown(body))
        .await
        .ok();
}

#[implement(super::Service)]
pub async fn send_message(&self, message_content: RoomMessageEventContent) -> Result {
    let user_id = &self.services.server_state.server_user;
    let room_id = self.get_admin_room().await?;

    self.respond_to_room(message_content, &room_id, user_id)
        .boxed()
        .await
}

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

    let server_user = &self.services.server_state.server_user;

    if self.services.server_state.user_is_local(user_id) {
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

pub async fn create_admin_room(services: &Services) -> Result {
    let room_id = RoomId::new(services.server_state.server_name());
    let room_version = &services.server.config.default_room_version;

    let _short_id = services.rooms.short.get_or_create_shortroomid(&room_id).await;

    let state_lock = services.rooms.state.mutex.lock(&room_id).await;

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
