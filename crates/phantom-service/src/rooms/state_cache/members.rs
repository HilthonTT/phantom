use super::*;

impl Service {
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn user_sees_user(&self, user_a: &UserId, user_b: &UserId) -> bool {
        let get_shared_rooms = self.get_shared_rooms(user_a, user_b);

        pin_mut!(get_shared_rooms);
        get_shared_rooms.next().await.is_some()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn get_shared_rooms<'a>(
        &'a self,
        user_a: &'a UserId,
        user_b: &'a UserId,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let a = self.rooms_joined(user_a);
        let b = self.rooms_joined(user_b);

        set::intersection_sorted_stream2(a, b)
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_joined
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn room_joined_count(&self, room_id: &RoomId) -> Result<u64> {
        self.db.roomid_joinedcount.get(room_id).await.deserialized()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn room_invited_count(&self, room_id: &RoomId) -> Result<u64> {
        self.db
            .roomid_invitedcount
            .get(room_id)
            .await
            .deserialized()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn local_users_in_room<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        self.room_members(room_id)
            .ready_filter(|user| self.services.server_state.user_is_local(user))
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub fn active_local_users_in_room<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        self.local_users_in_room(room_id)
            .filter(|user| self.services.users.is_active(user))
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_useroncejoined<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = once_joined_prefix(room_id);
        self.db
            .roomuseroncejoinedids
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members_invited<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_invitecount
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_members_knocked<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a UserId> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db
            .roomuserid_knockedcount
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, user_id): (Ignore, &str)| {
                <&UserId>::try_from(user_id).expect("valid user id in db")
            })
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_invite_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db
            .roomuserid_invitecount
            .qry(&key)
            .await
            .deserialized()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_knock_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db
            .roomuserid_knockedcount
            .qry(&key)
            .await
            .deserialized()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get_left_count(&self, room_id: &RoomId, user_id: &UserId) -> Result<u64> {
        let key = (room_id, user_id);
        self.db.roomuserid_leftcount.qry(&key).await.deserialized()
    }
}
