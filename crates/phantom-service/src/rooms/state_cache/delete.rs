use ruma::{OwnedServerName, OwnedUserId};

use super::*;

impl Service {
    #[tracing::instrument(skip(self), level = "debug")]
    pub(in crate::rooms) async fn delete_room_memberships(&self, room_id: &RoomId, force: bool) {
        let servers: Vec<OwnedServerName> = self
            .room_servers(room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        for server in &servers {
            self.db.roomserverids.del((room_id, server)).ok();
            self.db.serverroomids.del((server, room_id)).ok();
        }

        let memberships = [
            (&self.db.roomuserid_joined, &self.db.userroomid_joined),
            (
                &self.db.roomuserid_invitecount,
                &self.db.userroomid_invitestate,
            ),
            (
                &self.db.roomuserid_knockedcount,
                &self.db.userroomid_knockedstate,
            ),
        ];

        for (by_room, by_user) in memberships {
            let users = room_users(by_room, room_id).await;

            for user_id in &users {
                by_room.del((room_id, user_id)).ok();
                by_user.del((user_id, room_id)).ok();
            }
        }

        let left = room_users(&self.db.roomuserid_leftcount, room_id).await;

        for user_id in &left {
            if !force && self.services.server_state.user_is_local(user_id) {
                continue;
            }

            self.db.roomuserid_leftcount.del((room_id, user_id)).ok();
            self.db.userroomid_leftstate.del((user_id, room_id)).ok();
        }

        self.db
            .roomuseroncejoinedids
            .del_prefix(&once_joined_prefix(room_id))
            .await;

        self.db.roomid_joinedcount.remove(room_id).ok();
        self.db.roomid_invitedcount.remove(room_id).ok();
        self.db.roomid_inviteviaservers.remove(room_id).ok();

        self.appservice_in_room_cache
            .write()
            .expect("locked for writing")
            .remove(room_id);
    }
}

async fn room_users(map: &Arc<Map>, room_id: &RoomId) -> Vec<OwnedUserId> {
    let prefix = (room_id, Interfix);

    map.keys_prefix(&prefix)
        .ignore_err()
        .map(|(_, user_id): (Ignore, &str)| {
            <&UserId>::try_from(user_id)
                .expect("valid user id in db")
                .to_owned()
        })
        .collect()
        .await
}
