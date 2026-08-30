//! Reading a room's state.
//!
//! [`state_compressor`] can hand back the set of events that make up a room's
//! state at some version, and [`short`] can turn the short ids in it back into
//! event ids. This is what sits on top of the pair: everything that wants to
//! know something about a room asks it here, by state key, by type, or as a
//! whole.
//!
//! Two families of method, differing only in where the version comes from.
//! `room_state_*` in [`room_state`] takes a room and reads its current state;
//! `state_*` in [`state`] takes a shortstatehash and reads the state at that
//! version, which is what answers a question about a room as it was at some
//! event rather than as it is now. The former is the latter with the room's
//! current version looked up first.
//!
//! The visibility checks in [`user_can`] and [`server_can`] are here for the
//! same reason: `history_visibility` is state, and whether someone may see an
//! event turns on that state as it was at that event, not as it is now.
//!
//! [`short`]: crate::rooms::short
//! [`state_compressor`]: crate::rooms::state_compressor

mod room_state;
mod server_can;
mod state;
mod user_can;

use std::sync::Arc;

use phantom_core::{Result, err};
use phantom_database::Map;
use ruma::{
    EventEncryptionAlgorithm, JsOption, OwnedRoomAliasId, RoomId, UserId,
    events::{
        StateEventType,
        room::{
            avatar::RoomAvatarEventContent,
            canonical_alias::RoomCanonicalAliasEventContent,
            create::RoomCreateEventContent,
            encryption::RoomEncryptionEventContent,
            guest_access::{GuestAccess, RoomGuestAccessEventContent},
            history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
            join_rules::{JoinRule, RoomJoinRulesEventContent},
            member::RoomMemberEventContent,
            name::RoomNameEventContent,
            topic::RoomTopicEventContent,
        },
    },
    room::RoomType,
};

use crate::{Dep, rooms};

pub struct Service {
    services: Services,
    db: Data,
}

struct Services {
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    state_compressor: Dep<rooms::state_compressor::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

struct Data {
    shorteventid_shortstatehash: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                state_compressor: args
                    .depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
            db: Data {
                shorteventid_shortstatehash: args.db["shorteventid_shortstatehash"].clone(),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// The room's name, from `m.room.name`.
    pub async fn get_name(&self, room_id: &RoomId) -> Result<String> {
        self.room_state_get_content(room_id, &StateEventType::RoomName, "")
            .await
            .map(|c: RoomNameEventContent| c.name)
    }

    /// The room's avatar.
    ///
    /// [`JsOption`] rather than [`Option`] because the event distinguishes a
    /// room that never had an avatar from one whose avatar was explicitly
    /// cleared, and a client is shown different things for the two.
    pub async fn get_avatar(&self, room_id: &RoomId) -> JsOption<RoomAvatarEventContent> {
        let content = self
            .room_state_get_content(room_id, &StateEventType::RoomAvatar, "")
            .await
            .ok();

        JsOption::from_option(content)
    }

    /// A user's membership event content in the room's current state.
    pub async fn get_member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<RoomMemberEventContent> {
        self.room_state_get_content(room_id, &StateEventType::RoomMember, user_id.as_str())
            .await
    }

    /// Whether the room's content can be read without joining it.
    pub async fn is_world_readable(&self, room_id: &RoomId) -> bool {
        self.room_state_get_content(room_id, &StateEventType::RoomHistoryVisibility, "")
            .await
            .is_ok_and(|c: RoomHistoryVisibilityEventContent| {
                c.history_visibility == HistoryVisibility::WorldReadable
            })
    }

    /// Whether guests may join the room.
    pub async fn guest_can_join(&self, room_id: &RoomId) -> bool {
        self.room_state_get_content(room_id, &StateEventType::RoomGuestAccess, "")
            .await
            .is_ok_and(|c: RoomGuestAccessEventContent| c.guest_access == GuestAccess::CanJoin)
    }

    /// The room's primary alias, from `m.room.canonical_alias`.
    pub async fn get_canonical_alias(&self, room_id: &RoomId) -> Result<OwnedRoomAliasId> {
        self.room_state_get_content(room_id, &StateEventType::RoomCanonicalAlias, "")
            .await
            .and_then(|c: RoomCanonicalAliasEventContent| {
                c.alias
                    .ok_or_else(|| err!(Request(NotFound("No alias found in event content."))))
            })
    }

    /// The room's topic.
    pub async fn get_room_topic(&self, room_id: &RoomId) -> Result<String> {
        self.room_state_get_content(room_id, &StateEventType::RoomTopic, "")
            .await
            .map(|c: RoomTopicEventContent| c.topic)
    }

    /// The room's join rule, defaulting to `Invite` where there is no valid
    /// `m.room.join_rules` — which is the closed end of the range, so a room
    /// whose state cannot be read is not thereby thrown open.
    pub async fn get_join_rules(&self, room_id: &RoomId) -> JoinRule {
        self.room_state_get_content(room_id, &StateEventType::RoomJoinRules, "")
            .await
            .map_or(JoinRule::Invite, |c: RoomJoinRulesEventContent| c.join_rule)
    }

    /// The room's type, where it has one — a space, rather than a room.
    pub async fn get_room_type(&self, room_id: &RoomId) -> Result<RoomType> {
        self.room_state_get_content(room_id, &StateEventType::RoomCreate, "")
            .await
            .and_then(|content: RoomCreateEventContent| {
                content
                    .room_type
                    .ok_or_else(|| err!(Request(NotFound("No type found in event content"))))
            })
    }

    /// The room's encryption algorithm, where the room is encrypted.
    pub async fn get_room_encryption(&self, room_id: &RoomId) -> Result<EventEncryptionAlgorithm> {
        self.room_state_get_content(room_id, &StateEventType::RoomEncryption, "")
            .await
            .map(|content: RoomEncryptionEventContent| content.algorithm)
    }

    /// Whether the room is encrypted at all.
    pub async fn is_encrypted_room(&self, room_id: &RoomId) -> bool {
        self.room_state_get(room_id, &StateEventType::RoomEncryption, "")
            .await
            .is_ok()
    }
}
