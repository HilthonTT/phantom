use std::{cmp::Reverse, sync::Arc};

use futures::{FutureExt, Stream, StreamExt, TryFutureExt, future::join, stream::FuturesUnordered};
use phantom_core::{Err, Result, err, implement, stream::TryIgnore};
use phantom_database::Map;
use ruma::{
    OwnedRoomId, RoomId, UInt,
    api::client::room::Visibility,
    directory::{Filter, PublicRoomsChunk, PublicRoomsChunkInit, RoomTypeFilter},
};

use crate::{Dep, rooms};

pub struct Service {
    db: Data,
    services: Services,
}

struct Services {
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
}

pub struct PublicRoomsPage {
    pub chunk: Vec<PublicRoomsChunk>,
    pub prev_batch: Option<String>,
    pub next_batch: Option<String>,
    pub total_room_count_estimate: Option<UInt>,
}

const DEFAULT_LIMIT: usize = 10;

struct Data {
    publicroomids: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                publicroomids: args.db["publicroomids"].clone(),
            },
            services: Services {
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub fn set_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.insert(room_id, [])
}

#[implement(Service)]
pub fn set_not_public(&self, room_id: &RoomId) -> Result {
    self.db.publicroomids.remove(room_id)
}

#[implement(Service)]
pub fn public_rooms(&self) -> impl Stream<Item = &RoomId> + Send {
    self.db
        .publicroomids
        .keys::<&str>()
        .map(|room_id| {
            room_id.and_then(|room_id| {
                <&RoomId>::try_from(room_id)
                    .map_err(|e| err!(Database("Invalid room id in publicroomids: {e}")))
            })
        })
        .ignore_err()
}

#[implement(Service)]
pub async fn is_public_room(&self, room_id: &RoomId) -> bool {
    self.visibility(room_id).await == Visibility::Public
}

#[implement(Service)]
pub async fn visibility(&self, room_id: &RoomId) -> Visibility {
    if self.db.publicroomids.get(room_id).await.is_ok() {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

#[implement(Service)]
pub async fn public_rooms_page(
    &self,
    limit: Option<UInt>,
    since: Option<&str>,
    filter: &Filter,
) -> Result<PublicRoomsPage> {
    let limit = limit
        .and_then(|limit| usize::try_from(limit).ok())
        .unwrap_or(DEFAULT_LIMIT)
        // limit=0 would return next_batch pointing at the same page forever.
        .max(1);

    let offset = match since {
        Some(since) => parse_since(since, limit)?,
        None => 0,
    };

    let search_term = filter.generic_search_term.as_deref().map(str::to_lowercase);

    let room_ids: Vec<OwnedRoomId> = self.public_rooms().map(ToOwned::to_owned).collect().await;

    let chunks: FuturesUnordered<_> = room_ids
        .into_iter()
        .map(|room_id| self.public_rooms_chunk(room_id))
        .collect();

    let mut rooms: Vec<PublicRoomsChunk> = chunks
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter(|chunk| matches_room_types(chunk, filter))
        .filter(|chunk| {
            search_term
                .as_deref()
                .is_none_or(|term| matches_search_term(chunk, term))
        })
        .collect();

    rooms.sort_by_key(|chunk| Reverse(chunk.num_joined_members));

    let total_room_count_estimate = UInt::try_from(rooms.len()).ok();

    let chunk: Vec<_> = rooms.into_iter().skip(offset).take(limit).collect();

    let prev_batch = (offset != 0).then(|| format!("p{offset}"));
    let next_batch = (chunk.len() >= limit).then(|| format!("n{}", offset.saturating_add(limit)));

    Ok(PublicRoomsPage {
        chunk,
        prev_batch,
        next_batch,
        total_room_count_estimate,
    })
}

#[implement(Service)]
async fn public_rooms_chunk(&self, room_id: OwnedRoomId) -> PublicRoomsChunk {
    let state = &self.services.state_accessor;

    let summary = join(
        async {
            (
                state.get_name(&room_id).await.ok(),
                state.get_room_type(&room_id).await.ok(),
                state.get_canonical_alias(&room_id).await.ok(),
                state
                    .get_avatar(&room_id)
                    .await
                    .into_option()
                    .and_then(|avatar| avatar.url),
                state.get_room_topic(&room_id).await.ok(),
            )
        },
        async {
            (
                state.is_world_readable(&room_id).await,
                state
                    .get_join_rules(&room_id)
                    .map(|join_rule| join_rule.kind())
                    .await,
                state.guest_can_join(&room_id).await,
                self.services
                    .state_cache
                    .room_joined_count(&room_id)
                    .map_ok(|count| UInt::try_from(count).unwrap_or_default())
                    .await
                    .unwrap_or_default(),
            )
        },
    )
    .boxed();

    let (
        (name, room_type, canonical_alias, avatar_url, topic),
        (world_readable, join_rule, guest_can_join, num_joined_members),
    ) = summary.await;

    let mut chunk = PublicRoomsChunk::from(PublicRoomsChunkInit {
        num_joined_members,
        room_id,
        world_readable,
        guest_can_join,
    });

    chunk.name = name;
    chunk.room_type = room_type;
    chunk.canonical_alias = canonical_alias;
    chunk.avatar_url = avatar_url;
    chunk.topic = topic;
    chunk.join_rule = join_rule;

    chunk
}

fn parse_since(since: &str, limit: usize) -> Result<usize> {
    let (backwards, offset) = match since.split_at_checked(1) {
        Some(("n", offset)) => (false, offset),
        Some(("p", offset)) => (true, offset),
        _ => return Err!(Request(InvalidParam("Invalid `since` token."))),
    };

    let offset: usize = offset
        .parse()
        .map_err(|_| err!(Request(InvalidParam("Invalid `since` token."))))?;

    Ok(if backwards {
        offset.saturating_sub(limit)
    } else {
        offset
    })
}

fn matches_room_types(chunk: &PublicRoomsChunk, filter: &Filter) -> bool {
    filter.room_types.is_empty()
        || filter
            .room_types
            .contains(&RoomTypeFilter::from(chunk.room_type.clone()))
}

fn matches_search_term(chunk: &PublicRoomsChunk, term: &str) -> bool {
    let contains = |value: &str| value.to_lowercase().contains(term);

    chunk.name.as_deref().is_some_and(contains)
        || chunk.topic.as_deref().is_some_and(contains)
        || chunk
            .canonical_alias
            .as_ref()
            .is_some_and(|alias| contains(alias.as_str()))
}
