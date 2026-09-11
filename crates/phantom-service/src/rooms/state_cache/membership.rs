use super::*;

impl Service {
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            %room_id,
            %user_id,
            %sender,
            ?membership_event,
        ),
    )]
    #[allow(clippy::too_many_arguments)]
    pub async fn update_membership(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
        membership_event: RoomMemberEventContent,
        sender: &UserId,
        last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
        invite_via: Option<Vec<OwnedServerName>>,
        update_joined_count: bool,
    ) -> Result<()> {
        let membership = membership_event.membership;

        if !self.services.server_state.user_is_local(user_id)
            && !self.services.users.exists(user_id).await
        {
            self.services.users.create(user_id, None)?;
        }

        match &membership {
            MembershipState::Join => {
                if !self.once_joined(user_id, room_id).await {
                    self.mark_as_once_joined(user_id, room_id);

                    if let Ok(Some(predecessor)) = self
                        .services
                        .state_accessor
                        .room_state_get_content(room_id, &StateEventType::RoomCreate, "")
                        .await
                        .map(|content: RoomCreateEventContent| content.predecessor)
                    {
                        self.copy_room_settings(&predecessor.room_id, room_id, user_id)
                            .await?;
                    }
                }

                self.mark_as_joined(user_id, room_id);
            }
            MembershipState::Invite => {
                if self.services.users.user_is_ignored(sender, user_id).await {
                    return Ok(());
                }

                self.mark_as_invited(user_id, room_id, last_state, invite_via)
                    .await;
            }
            MembershipState::Leave | MembershipState::Ban => {
                self.mark_as_left(user_id, room_id);

                if self.services.server_state.user_is_local(user_id)
                    && (self.services.server.config.rooms.forget_forced_upon_leave
                        || self.services.metadata.is_banned(room_id).await
                        || self.services.metadata.is_disabled(room_id).await)
                {
                    self.forget(room_id, user_id);
                }
            }
            MembershipState::Knock => {
                self.mark_as_knocked(user_id, room_id, last_state);
            }
            _ => {}
        }

        if update_joined_count {
            self.update_joined_count(room_id).await;
        }

        Ok(())
    }

    async fn copy_room_settings(
        &self,
        predecessor: &RoomId,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<()> {
        if let Ok(tag_event) = self
            .services
            .account_data
            .get_room::<serde_json::Value>(predecessor, user_id, RoomAccountDataEventType::Tag)
            .await
        {
            self.services
                .account_data
                .update(
                    Some(room_id),
                    user_id,
                    RoomAccountDataEventType::Tag,
                    &tag_event,
                )
                .await
                .ok();
        }

        if let Ok(mut direct_event) = self
            .services
            .account_data
            .get_global::<DirectEvent>(user_id, GlobalAccountDataEventType::Direct)
            .await
        {
            let mut room_ids_updated = false;
            for room_ids in direct_event.content.0.values_mut() {
                if room_ids.iter().any(|r| r == predecessor) {
                    room_ids.push(room_id.to_owned());
                    room_ids_updated = true;
                }
            }

            if room_ids_updated {
                self.services
                    .account_data
                    .update(
                        None,
                        user_id,
                        GlobalAccountDataEventType::Direct.to_string().into(),
                        &serde_json::to_value(&direct_event).expect("to json always works"),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_joined(&self, user_id: &UserId, room_id: &RoomId) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db.userroomid_joined.insert(&userroom_id, []).ok();
        self.db.roomuserid_joined.insert(&roomuser_id, []).ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Joined);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_left(&self, user_id: &UserId, room_id: &RoomId) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        let leftstate = Vec::<Raw<AnySyncStateEvent>>::new();

        self.db
            .userroomid_leftstate
            .raw_put(&userroom_id, Json(leftstate))
            .ok();
        self.db
            .roomuserid_leftcount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Left);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn mark_as_knocked(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        knocked_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
    ) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db
            .userroomid_knockedstate
            .raw_put(&userroom_id, Json(knocked_state.unwrap_or_default()))
            .ok();
        self.db
            .roomuserid_knockedcount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Knocked);
        self.db.roomid_inviteviaservers.remove(room_id).ok();
    }

    #[tracing::instrument(level = "debug", skip(self, last_state, invite_via))]
    pub async fn mark_as_invited(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        last_state: Option<Vec<Raw<AnyStrippedStateEvent>>>,
        invite_via: Option<Vec<OwnedServerName>>,
    ) {
        let userroom_id = serialize_key((user_id, room_id)).expect("failed to serialize key");
        let roomuser_id = serialize_key((room_id, user_id)).expect("failed to serialize key");

        self.db
            .userroomid_invitestate
            .raw_put(&userroom_id, Json(last_state.unwrap_or_default()))
            .ok();
        self.db
            .roomuserid_invitecount
            .raw_aput::<8, _, _>(&roomuser_id, self.next_count())
            .ok();

        self.clear_other_memberships(&userroom_id, &roomuser_id, Membership::Invited);

        if let Some(servers) = invite_via.filter(is_not_empty!()) {
            self.add_servers_invite_via(room_id, servers).await;
        }
    }

    #[tracing::instrument(level = "debug", skip(self))]
    fn mark_as_once_joined(&self, user_id: &UserId, room_id: &RoomId) {
        let key = once_joined_key(room_id, user_id);
        self.db.roomuseroncejoinedids.put_raw(key, []).ok();
    }

    fn clear_other_memberships(&self, userroom_id: &[u8], roomuser_id: &[u8], keep: Membership) {
        if keep != Membership::Joined {
            self.db.userroomid_joined.remove(userroom_id).ok();
            self.db.roomuserid_joined.remove(roomuser_id).ok();
        }

        if keep != Membership::Invited {
            self.db.userroomid_invitestate.remove(userroom_id).ok();
            self.db.roomuserid_invitecount.remove(roomuser_id).ok();
        }

        if keep != Membership::Left {
            self.db.userroomid_leftstate.remove(userroom_id).ok();
            self.db.roomuserid_leftcount.remove(roomuser_id).ok();
        }

        if keep != Membership::Knocked {
            self.db.userroomid_knockedstate.remove(userroom_id).ok();
            self.db.roomuserid_knockedcount.remove(roomuser_id).ok();
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn forget(&self, room_id: &RoomId, user_id: &UserId) {
        self.db.userroomid_leftstate.del((user_id, room_id)).ok();
        self.db.roomuserid_leftcount.del((room_id, user_id)).ok();
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn invite_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_invitestate, user_id, room_id)
            .await
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn knock_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_knockedstate, user_id, room_id)
            .await
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn left_state(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        self.stripped_state(&self.db.userroomid_leftstate, user_id, room_id)
            .await
    }

    async fn stripped_state(
        &self,
        map: &Arc<Map>,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Result<Vec<Raw<AnyStrippedStateEvent>>> {
        let key = (user_id, room_id);
        map.qry(&key)
            .await
            .deserialized()
            .and_then(|val: Raw<Vec<Raw<AnyStrippedStateEvent>>>| {
                val.deserialize().map_err(Into::into)
            })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub async fn once_joined(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = once_joined_key(room_id, user_id);
        self.db.roomuseroncejoinedids.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_joined<'a>(&'a self, user_id: &'a UserId, room_id: &'a RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_joined.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_knocked<'a>(&'a self, user_id: &'a UserId, room_id: &'a RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_knockedstate.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_invited(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_invitestate.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn is_left(&self, user_id: &UserId, room_id: &RoomId) -> bool {
        let key = (user_id, room_id);
        self.db.userroomid_leftstate.qry(&key).await.is_ok()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn user_membership(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
    ) -> Option<MembershipState> {
        let states = join5(
            self.is_joined(user_id, room_id),
            self.is_left(user_id, room_id),
            self.is_knocked(user_id, room_id),
            self.is_invited(user_id, room_id),
            self.once_joined(user_id, room_id),
        )
        .await;

        match states {
            (true, ..) => Some(MembershipState::Join),
            (_, true, ..) => Some(MembershipState::Leave),
            (_, _, true, ..) => Some(MembershipState::Knock),
            (_, _, _, true, ..) => Some(MembershipState::Invite),
            (false, false, false, false, true) => Some(MembershipState::Ban),
            _ => None,
        }
    }
}
