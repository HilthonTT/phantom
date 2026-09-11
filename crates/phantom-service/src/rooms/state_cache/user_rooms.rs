use super::*;

impl Service {
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

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn rooms_invited<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_invitestate, user_id)
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub fn rooms_knocked<'a>(
        &'a self,
        user_id: &'a UserId,
    ) -> impl Stream<Item = StrippedStateEventItem> + Send + 'a {
        stripped_rooms(&self.db.userroomid_knockedstate, user_id)
    }

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
