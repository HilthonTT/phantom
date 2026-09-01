use super::*;

impl Service {
    /// Every server with a member in this room.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn room_servers<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a ServerName> + Send + 'a {
        let prefix = (room_id, Interfix);
        self.db.roomserverids.keys_prefix(&prefix).ignore_err().map(
            |(_, server): (Ignore, &str)| {
                <&ServerName>::try_from(server).expect("valid server name in db")
            },
        )
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn server_in_room<'a>(&'a self, server: &'a ServerName, room_id: &'a RoomId) -> bool {
        let key = (server, room_id);
        self.db.serverroomids.qry(&key).await.is_ok()
    }

    /// Every room this server knows a given server to be in.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn server_rooms<'a>(
        &'a self,
        server: &'a ServerName,
    ) -> impl Stream<Item = &'a RoomId> + Send + 'a {
        let prefix = (server, Interfix);
        self.db.serverroomids.keys_prefix(&prefix).ignore_err().map(
            |(_, room_id): (Ignore, &str)| {
                <&RoomId>::try_from(room_id).expect("valid room id in db")
            },
        )
    }

    /// Whether a server shares any room with a user, which is what entitles it
    /// to see that user's profile and device list.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn server_sees_user(&self, server: &ServerName, user_id: &UserId) -> bool {
        self.server_rooms(server)
            .any(|room_id| self.is_joined(user_id, room_id))
            .await
    }

    /// The servers an invite named as able to serve the room, which is how a
    /// user joins a room this server has never otherwise heard of.
    #[tracing::instrument(skip(self), level = "debug")]
    pub fn servers_invite_via<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a ServerName> + Send + 'a {
        type KeyVal<'a> = (Ignore, Vec<&'a str>);

        self.db
            .roomid_inviteviaservers
            .stream_prefix(room_id)
            .ignore_err()
            .map(|(_, servers): KeyVal<'_>| servers)
            .flat_map(|servers| {
                servers
                    .into_iter()
                    .map(|server| <&ServerName>::try_from(server).expect("valid server name in db"))
                    .stream()
            })
    }

    /// Adds to the servers recorded by [`servers_invite_via`], keeping the
    /// list sorted and free of duplicates.
    ///
    /// [`servers_invite_via`]: Self::servers_invite_via
    #[tracing::instrument(level = "debug", skip(self, servers))]
    pub async fn add_servers_invite_via(&self, room_id: &RoomId, servers: Vec<OwnedServerName>) {
        let mut servers: Vec<_> = self
            .servers_invite_via(room_id)
            .map(ToOwned::to_owned)
            .chain(iter(servers))
            .collect()
            .await;

        servers.sort_unstable();
        servers.dedup();

        let servers = servers
            .iter()
            .map(|server| server.as_bytes())
            .collect::<Vec<_>>()
            .join(&[0xFF][..]);

        self.db
            .roomid_inviteviaservers
            .insert(room_id.as_bytes(), &servers)
            .ok();
    }

    /// Up to five servers likely to still be in the room some time from now,
    /// which is what a room's permalinks are built from.
    ///
    /// See <https://spec.matrix.org/latest/appendices/#routing>.
    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn servers_route_via(&self, room_id: &RoomId) -> Result<Vec<OwnedServerName>> {
        let most_powerful_user_server = self
            .services
            .state_accessor
            .room_state_get_content(room_id, &StateEventType::RoomPowerLevels, "")
            .await
            .map(|content: RoomPowerLevelsEventContent| {
                content
                    .users
                    .iter()
                    .max_by_key(|(_, power)| *power)
                    .and_then(|x| (x.1 >= &int!(50)).then_some(x))
                    .map(|(user, _power)| user.server_name().to_owned())
            });

        let mut counts: HashMap<OwnedServerName, usize> = HashMap::new();
        self.room_members(room_id)
            .ready_for_each(|user| {
                *counts.entry(user.server_name().to_owned()).or_default() += 1;
            })
            .await;

        let mut by_members: Vec<_> = counts.into_iter().collect();
        by_members.sort_unstable_by_key(|(_, users)| *users);

        let mut servers: Vec<OwnedServerName> = by_members
            .into_iter()
            .map(|(server, _)| server)
            .rev()
            .take(5)
            .collect();

        if let Ok(Some(server)) = most_powerful_user_server {
            servers.insert(0, server);
            servers.truncate(5);
        }

        Ok(servers)
    }
}
