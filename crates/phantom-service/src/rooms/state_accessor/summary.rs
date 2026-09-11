use futures::{FutureExt, future::join5};
use phantom_core::{Result, implement};
use ruma::{
    RoomId, UInt,
    room::{JoinRuleSummary, RoomSummary},
};

#[implement(super::Service)]
pub async fn room_summary(&self, room_id: &RoomId) -> RoomSummary {
    let name = self.get_name(room_id).map(Result::ok);
    let topic = self.get_room_topic(room_id).map(Result::ok);
    let canonical_alias = self.get_canonical_alias(room_id).map(Result::ok);
    let room_type = self.get_room_type(room_id).map(Result::ok);
    let encryption = self.get_room_encryption(room_id).map(Result::ok);

    let (name, topic, canonical_alias, room_type, encryption) =
        join5(name, topic, canonical_alias, room_type, encryption).await;

    let avatar_url = self.get_avatar(room_id).await.into_option();

    let join_rule: JoinRuleSummary = self.get_join_rules(room_id).await.into();

    let num_joined_members = self
        .services
        .state_cache
        .room_joined_count(room_id)
        .await
        .unwrap_or(0);

    let mut summary = RoomSummary::new(
        room_id.to_owned(),
        join_rule,
        self.guest_can_join(room_id).await,
        UInt::new_saturating(num_joined_members),
        self.is_world_readable(room_id).await,
    );

    summary.canonical_alias = canonical_alias;
    summary.name = name;
    summary.topic = topic;
    summary.avatar_url = avatar_url.and_then(|avatar| avatar.url);
    summary.room_type = room_type;
    summary.encryption = encryption;
    summary.room_version = self.services.state.get_room_version(room_id).await.ok();

    summary
}
