mod room_state;
mod server_can;
mod state;
mod summary;
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
    pub async fn get_name(&self, room_id: &RoomId) -> Result<String> {
        self.room_state_get_content(room_id, &StateEventType::RoomName, "")
            .await
            .map(|c: RoomNameEventContent| c.name)
    }

    pub async fn get_avatar(&self, room_id: &RoomId) -> JsOption<RoomAvatarEventContent> {
        let content = self
            .room_state_get_content(room_id, &StateEventType::RoomAvatar, "")
            .await
            .ok();

        JsOption::from_option(content)
    }

    pub async fn get_member(
        &self,
        room_id: &RoomId,
        user_id: &UserId,
    ) -> Result<RoomMemberEventContent> {
        self.room_state_get_content(room_id, &StateEventType::RoomMember, user_id.as_str())
            .await
    }

    pub async fn is_world_readable(&self, room_id: &RoomId) -> bool {
        self.room_state_get_content(room_id, &StateEventType::RoomHistoryVisibility, "")
            .await
            .is_ok_and(|c: RoomHistoryVisibilityEventContent| {
                c.history_visibility == HistoryVisibility::WorldReadable
            })
    }

    pub async fn guest_can_join(&self, room_id: &RoomId) -> bool {
        self.room_state_get_content(room_id, &StateEventType::RoomGuestAccess, "")
            .await
            .is_ok_and(|c: RoomGuestAccessEventContent| c.guest_access == GuestAccess::CanJoin)
    }

    pub async fn get_canonical_alias(&self, room_id: &RoomId) -> Result<OwnedRoomAliasId> {
        self.room_state_get_content(room_id, &StateEventType::RoomCanonicalAlias, "")
            .await
            .and_then(|c: RoomCanonicalAliasEventContent| {
                c.alias
                    .ok_or_else(|| err!(Request(NotFound("No alias found in event content."))))
            })
    }

    pub async fn get_room_topic(&self, room_id: &RoomId) -> Result<String> {
        self.room_state_get_content(room_id, &StateEventType::RoomTopic, "")
            .await
            .map(|c: RoomTopicEventContent| c.topic)
    }

    pub async fn get_join_rules(&self, room_id: &RoomId) -> JoinRule {
        self.room_state_get_content(room_id, &StateEventType::RoomJoinRules, "")
            .await
            .map_or(JoinRule::Invite, |c: RoomJoinRulesEventContent| c.join_rule)
    }

    pub async fn get_room_type(&self, room_id: &RoomId) -> Result<RoomType> {
        self.room_state_get_content(room_id, &StateEventType::RoomCreate, "")
            .await
            .and_then(|content: RoomCreateEventContent| {
                content
                    .room_type
                    .ok_or_else(|| err!(Request(NotFound("No type found in event content"))))
            })
    }

    pub async fn get_room_encryption(&self, room_id: &RoomId) -> Result<EventEncryptionAlgorithm> {
        self.room_state_get_content(room_id, &StateEventType::RoomEncryption, "")
            .await
            .map(|content: RoomEncryptionEventContent| content.algorithm)
    }

    pub async fn is_encrypted_room(&self, room_id: &RoomId) -> bool {
        self.room_state_get(room_id, &StateEventType::RoomEncryption, "")
            .await
            .is_ok()
    }
}
