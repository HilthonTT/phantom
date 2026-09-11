mod hierarchy;
mod token;

use std::{
    fmt::Write,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures::StreamExt;
use lru_cache::LruCache;
use phantom_core::{Err, Result, debug, implement, math::usize_from_f64, trace};
use ruma::{
    OwnedRoomId, OwnedServerName, RoomId, ServerName, UserId,
    api::{client::space::SpaceHierarchyRoomsChunk, federation},
    events::{
        StateEventType,
        space::child::{HierarchySpaceChildEvent, SpaceChildOrd},
    },
    room::{JoinRuleSummary, RoomSummary},
    serde::Raw,
};

pub use self::hierarchy::PagedHierarchy;
use crate::{
    Dep,
    moderation::{self, Restriction},
    rooms, server_state,
};

pub struct Service {
    cache: Mutex<LruCache<CacheKey, SpaceHierarchyRoomsChunk>>,
    services: Services,
}

struct Services {
    federation: Dep<crate::federation::Service>,
    metadata: Dep<rooms::metadata::Service>,
    moderation: Dep<moderation::Service>,
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

type CacheKey = (OwnedRoomId, bool);

#[derive(Clone, Copy, Debug)]
pub enum Asker<'a> {
    User(&'a UserId),

    Server(&'a ServerName),
}

#[derive(Clone, Debug)]
pub enum SummaryAccessibility {
    Accessible(Box<SpaceHierarchyRoomsChunk>),
    Inaccessible,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;
        let capacity = f64::from(config.database.space_hierarchy_cache_capacity)
            * config.database.cache_capacity_modifier;

        Ok(Arc::new(Self {
            cache: LruCache::new(usize_from_f64(capacity)?).into(),
            services: Services {
                federation: args.depend::<crate::federation::Service>("federation"),
                metadata: args.depend::<rooms::metadata::Service>("rooms::metadata"),
                moderation: args.depend::<moderation::Service>("moderation"),
                server_state: args.depend::<server_state::Service>("server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
        }))
    }

    async fn clear_cache(&self) {
        self.cache.lock().expect("locked").clear();
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        let cache = self.cache.lock().expect("locked");

        writeln!(
            out,
            "space_hierarchy_cache: {} / {}",
            cache.len(),
            cache.capacity()
        )?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub async fn summary(
    &self,
    room_id: &RoomId,
    asker: Asker<'_>,
    via: &[OwnedServerName],
    suggested_only: bool,
) -> Option<SummaryAccessibility> {
    if self.services.metadata.exists(room_id).await {
        let summary = self.local_summary(room_id).await;

        return Some(self.accessibility(room_id, summary, asker).await);
    }

    let summary = self.remote_summary(room_id, via, suggested_only).await?;

    Some(self.accessibility(room_id, summary, asker).await)
}

#[implement(Service)]
async fn local_summary(&self, room_id: &RoomId) -> SpaceHierarchyRoomsChunk {
    let summary = self.services.state_accessor.room_summary(room_id).await;
    let children_state = self.children_state(room_id).await;

    SpaceHierarchyRoomsChunk::new(summary, children_state)
}

#[implement(Service)]
pub async fn children_state(&self, room_id: &RoomId) -> Vec<Raw<HierarchySpaceChildEvent>> {
    let Ok(shortstatehash) = self.services.state.get_room_shortstatehash(room_id).await else {
        return Vec::new();
    };

    let mut children: Vec<_> = self
        .services
        .state_accessor
        .state_keys_with_ids::<ruma::OwnedEventId>(shortstatehash, &StateEventType::SpaceChild)
        .filter_map(
            |(_, event_id)| async move { self.services.timeline.get_pdu(&event_id).await.ok() },
        )
        .map(phantom_core::matrix::PduEvent::into_stripped_spacechild_state_event)
        .filter_map(|raw| async move {
            let event = raw.deserialize().ok()?;

            (!event.content.via.is_empty()).then_some((event, raw))
        })
        .collect()
        .await;

    children.sort_by(|(a, _), (b, _)| a.cmp_space_child(b));

    children.into_iter().map(|(_, raw)| raw).collect()
}

#[implement(Service)]
async fn remote_summary(
    &self,
    room_id: &RoomId,
    via: &[OwnedServerName],
    suggested_only: bool,
) -> Option<SpaceHierarchyRoomsChunk> {
    if let Some(cached) = self.cached(room_id, suggested_only) {
        return Some(cached);
    }

    for server in self.candidates(room_id, via) {
        if self
            .services
            .moderation
            .forbids(&server, Restriction::Federation)
        {
            continue;
        }

        let mut request = federation::space::get_hierarchy::v1::Request::new(room_id.to_owned());
        request.suggested_only = suggested_only;

        let response = match self.services.federation.execute(&server, request).await {
            Ok(response) => response,
            Err(e) => {
                debug!(%server, %room_id, "Hierarchy request failed: {e}");
                continue;
            }
        };

        let chunk =
            SpaceHierarchyRoomsChunk::new(response.room.summary, response.room.children_state);

        for child in response.children {
            let room_id = child.room_id.clone();

            self.cache_summary(
                &room_id,
                suggested_only,
                SpaceHierarchyRoomsChunk::new(child, Vec::new()),
            );
        }

        self.cache_summary(room_id, suggested_only, chunk.clone());

        return Some(chunk);
    }

    trace!(%room_id, "No server could describe the room");

    None
}

#[implement(Service)]
fn candidates(&self, room_id: &RoomId, via: &[OwnedServerName]) -> Vec<OwnedServerName> {
    let own = room_id.server_name().map(ToOwned::to_owned);

    via.iter()
        .cloned()
        .chain(own)
        .filter(|server| !self.services.server_state.server_is_ours(server))
        .fold(Vec::new(), |mut servers, server| {
            if !servers.contains(&server) {
                servers.push(server);
            }

            servers
        })
}

#[implement(Service)]
pub fn forget(&self, room_id: &RoomId) {
    let mut cache = self.cache.lock().expect("locked");

    cache.remove(&(room_id.to_owned(), true));
    cache.remove(&(room_id.to_owned(), false));
}

#[implement(Service)]
fn cached(&self, room_id: &RoomId, suggested_only: bool) -> Option<SpaceHierarchyRoomsChunk> {
    self.cache
        .lock()
        .expect("locked")
        .get_mut(&(room_id.to_owned(), suggested_only))
        .cloned()
}

#[implement(Service)]
fn cache_summary(&self, room_id: &RoomId, suggested_only: bool, summary: SpaceHierarchyRoomsChunk) {
    self.cache
        .lock()
        .expect("locked")
        .insert((room_id.to_owned(), suggested_only), summary);
}

#[implement(Service)]
async fn accessibility(
    &self,
    room_id: &RoomId,
    summary: SpaceHierarchyRoomsChunk,
    asker: Asker<'_>,
) -> SummaryAccessibility {
    if self.accessible_to(room_id, &summary.summary, asker).await {
        SummaryAccessibility::Accessible(Box::new(summary))
    } else {
        SummaryAccessibility::Inaccessible
    }
}

#[implement(Service)]
pub async fn accessible_to(
    &self,
    room_id: &RoomId,
    summary: &RoomSummary,
    asker: Asker<'_>,
) -> bool {
    if self.is_in_room(room_id, asker).await {
        return true;
    }

    if summary.world_readable {
        return true;
    }

    match &summary.join_rule {
        JoinRuleSummary::Public | JoinRuleSummary::Knock => true,
        JoinRuleSummary::Restricted(restricted) | JoinRuleSummary::KnockRestricted(restricted) => {
            for allowed in &restricted.allowed_room_ids {
                if self.is_in_room(allowed, asker).await {
                    return true;
                }
            }

            matches!(summary.join_rule, JoinRuleSummary::KnockRestricted(_))
        }
        _ => false,
    }
}

#[implement(Service)]
async fn is_in_room(&self, room_id: &RoomId, asker: Asker<'_>) -> bool {
    match asker {
        Asker::User(user_id) => {
            self.services.state_cache.is_joined(user_id, room_id).await
                || self.services.state_cache.is_invited(user_id, room_id).await
        }
        Asker::Server(server) => {
            self.services
                .state_cache
                .server_in_room(server, room_id)
                .await
        }
    }
}

#[implement(Service)]
pub async fn federation_hierarchy(
    &self,
    room_id: &RoomId,
    server: &ServerName,
    suggested_only: bool,
) -> Result<federation::space::get_hierarchy::v1::Response> {
    let asker = Asker::Server(server);

    let Some(SummaryAccessibility::Accessible(parent)) =
        self.summary(room_id, asker, &[], suggested_only).await
    else {
        return Err!(Request(NotFound("The room is unknown to this server.")));
    };

    let mut children = Vec::new();
    let mut inaccessible = Vec::new();

    for (child, _) in self.children_of(&parent, suggested_only) {
        match self.summary(&child, asker, &[], suggested_only).await {
            Some(SummaryAccessibility::Accessible(chunk)) => children.push(chunk.summary),
            Some(SummaryAccessibility::Inaccessible) => inaccessible.push(child),

            None => {}
        }
    }

    let mut response = federation::space::get_hierarchy::v1::Response::new(
        federation::space::SpaceHierarchyParentSummary::new(parent.summary, parent.children_state),
    );

    response.children = children;
    response.inaccessible_children = inaccessible;

    Ok(response)
}

#[implement(Service)]
fn children_of(
    &self,
    summary: &SpaceHierarchyRoomsChunk,
    suggested_only: bool,
) -> Vec<(OwnedRoomId, Vec<OwnedServerName>)> {
    summary
        .children_state
        .iter()
        .filter_map(|raw| raw.deserialize().ok())
        .filter(|child| !suggested_only || child.content.suggested)
        .map(|child| (child.state_key, child.content.via))
        .collect()
}
