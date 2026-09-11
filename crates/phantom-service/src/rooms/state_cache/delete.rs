//! Dropping a room's membership indexes when the room itself is going.
//!
//! Every column here is indexed twice — once to walk a room's members, once
//! to walk a user's rooms — and only the first of each pair can be reached
//! from a room id. So the room-keyed half is read for the ids it holds before
//! anything is deleted, and those ids are what the user-keyed half is deleted
//! by. Reading first rather than deleting as the scan goes is what keeps the
//! two halves from parting company if the run is cut short: a key left in
//! both is a stale membership, a key left in one is an index that disagrees
//! with itself.

use ruma::{OwnedServerName, OwnedUserId};

use super::*;

impl Service {
    /// Drops every record of who was in a room and which servers it reached.
    ///
    /// `force` decides what happens to the local users who have left. Their
    /// leave record is what puts the room in the `leave` section of their
    /// sync, which is how a client learns the room is over rather than
    /// finding it silently gone; it is kept unless `force`. Remote users'
    /// leave records are always dropped — no client of this server reads
    /// them.
    ///
    /// The counts are removed rather than recomputed. [`update_joined_count`]
    /// would write zeroes back, and a room that is being purged should have
    /// no row at all.
    ///
    /// [`update_joined_count`]: Self::update_joined_count
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

/// The users a room-keyed membership column holds for one room.
///
/// Collected rather than streamed because the caller deletes out of this very
/// column as it goes, and a cursor is not the place to be standing while that
/// happens.
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
