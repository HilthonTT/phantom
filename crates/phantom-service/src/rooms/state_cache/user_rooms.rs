use super::*;

impl Service {
    /// Every room a user is joined to.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_joined<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let prefix = (user_id, Interfix);
        self.db
            .userroomid_joined
            .keys_prefix(&prefix)
            .ignore_err()
            .map(|(_, room_id): (Ignore, &str)| {
                <&RoomId>::try_from(room_id).expect("valid room id in db")
            })
    }

    /// Every room a user has been invited to, with the stripped state the
    /// client is shown for it.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_invited<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_invitestate, user_id)
    }

    /// Every room a user is knocking at, with its stripped state.
    #[tracing::instrument(skip(self), level = "trace")]
    pub fn rooms_knocked<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_knockedstate, user_id)
    }

    /// Every room a user has left.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_left<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = SyncStateEventItem> + Send + 'a {
        type KeyVal<'a> = ((Ignore, &'a str), Raw<Vec<Raw<AnySyncStateEvent>>>);

        let prefix = (user_id, Interfix);
        self.db
            .userroomid_leftstate
            .stream_prefix(&prefix)
            .ignore_err()
            .map(|((_, room_id), state): KeyVal<'_>| {
                let room_id = <&RoomId>::try_from(room_id).expect("valid room id in db");
                Ok((room_id.to_owned(), state.deserialize()?))
            })
            .ignore_err()
    }
}
