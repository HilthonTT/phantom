//! Spaces: rooms whose purpose is to hold other rooms.
//!
//! A space is an ordinary room with `type: m.space` in its create event, whose
//! `m.space.child` state names the rooms in it. There is no separate storage
//! here and no separate index — a space's children are its state, read like
//! any other state.
//!
//! What is here is the walk over that structure, which is harder than it
//! sounds for three reasons.
//!
//! **A child may live on another server.** The `m.space.child` event carries
//! `via` servers precisely because the space's own server may know nothing
//! about the room beyond its id. Summarizing such a child means a federation
//! request, so a summary fetched that way is cached; a local one is not, since
//! rebuilding it is a state read and caching it would only mean showing a
//! stale room name after a rename.
//!
//! **Not every child may be shown to everyone.** A hierarchy is answered for
//! somebody — a user over the client API, another server over federation — and
//! a room they could not peek or join is omitted rather than described. That
//! is [`accessible_to`], and it is evaluated per asker on every request rather
//! than cached, because the answer changes the moment someone joins a room.
//!
//! **The structure may not be a tree.** Nothing stops two spaces being each
//! other's child, and a walk that believed the structure was a tree would not
//! terminate. The walk in [`hierarchy`] carries the path it took and refuses
//! to descend into a room already on it.
//!
//! [`accessible_to`]: Service::accessible_to
//! [`hierarchy`]: Service::client_hierarchy

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
    /// Summaries fetched from other servers.
    ///
    /// Keyed by `suggested_only` as well as by room, because that flag is sent
    /// to the far end and decides which children come back in the summary; one
    /// cache entry answering both would hand a caller children it filtered
    /// out, or hide children it asked for.
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

/// Who a hierarchy is being answered for.
///
/// The two arms are not interchangeable: a user is in a room or is not, while
/// a server is in a room if any of its users are. Passing the wrong one would
/// show one user's rooms to a whole server, so the distinction is in the type
/// rather than left to a boolean beside a name.
#[derive(Clone, Copy, Debug)]
pub enum Asker<'a> {
    /// A user of this server, asking over the client API.
    User(&'a UserId),

    /// Another server, asking over federation.
    Server(&'a ServerName),
}

/// A room's summary, or the fact that it exists and may not be shown.
///
/// Distinct from the absence of a summary, which means the room could not be
/// described at all — nothing local, and no server willing to say. A caller
/// walking a hierarchy skips both, but a caller answering for one room owes
/// the asker a 403 for the first and a 404 for the second.
#[derive(Clone, Debug)]
pub enum SummaryAccessibility {
    /// Boxed because a summary is some 240 bytes and the other arm is empty;
    /// a walk moves one of these per room it visits.
    Accessible(Box<SpaceHierarchyRoomsChunk>),
    Inaccessible,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;
        let capacity =
            f64::from(config.space_hierarchy_cache_capacity) * config.cache_capacity_modifier;

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

/// Describes one room for a hierarchy, from local state or from a server that
/// knows it.
///
/// `via` are the servers the `m.space.child` event named, tried in order; the
/// room's own server is tried too, since a child event with a stale `via` is
/// common and the room id names a server by construction.
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

/// Describes a room this server is in, from its current state.
#[implement(Service)]
async fn local_summary(&self, room_id: &RoomId) -> SpaceHierarchyRoomsChunk {
    let summary = self.services.state_accessor.room_summary(room_id).await;
    let children_state = self.children_state(room_id).await;

    SpaceHierarchyRoomsChunk::new(summary, children_state)
}

/// The `m.space.child` events of a space, stripped for a hierarchy response.
///
/// Ordered as the spec orders children — by `order`, then by the child's
/// timestamp, then by room id — so that two servers answering for the same
/// space list it the same way, and so a client paginating gets a stable
/// sequence rather than whatever order the state happened to be read in.
///
/// A child whose `via` is empty is dropped: it names a room with no way to
/// reach it, which the spec says is not a child at all.
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

/// Asks the servers that might know the room to describe it.
///
/// The whole `/hierarchy` answer is taken, not just the room asked about: the
/// children summaries that come with it are the ones a walk is about to ask
/// for next, so caching them turns a request per child into one request per
/// space.
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

        // The children come back without children_state of their own, which is
        // exactly right: they are summaries, and a walk that descends into one
        // asks that room's own server for its children.
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

/// The servers to ask about a room, `via` first and the room's own last.
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

/// Drops any cached summary of a room, at both settings of `suggested_only`.
///
/// Called when the room's own `m.space.child` state changes: only summaries
/// fetched from elsewhere are cached, but a room joined after such a fetch has
/// one in the cache that its own state now contradicts.
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

/// Wraps a summary in whether the asker may be shown it.
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

/// Whether `asker` may be shown this room in a hierarchy.
///
/// The spec's rule, in the order that answers soonest: already in the room;
/// the room is world-readable; the room can be joined or knocked on by anyone;
/// or the room is restricted to members of a room the asker is already in.
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
            // Knock-restricted is knockable by anyone, but a server answering
            // for it still owes the narrower check first: the allowed rooms are
            // what the asker is likely to be in, and the check is local.
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

/// Whether the asker is in the room — joined or invited for a user, holding
/// any member for a server.
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

/// Answers `/_matrix/federation/v1/hierarchy` for a space this server holds.
///
/// One level only: the asking server walks the tree itself, descending through
/// the `children_state` of what it is given here, which is what stops a walk
/// from fanning out across servers on somebody else's behalf.
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
            // A child this server knows nothing about is left out entirely,
            // rather than reported as inaccessible: "we refuse to describe it"
            // and "we have never heard of it" are different answers, and only
            // the first belongs in `inaccessible_children`.
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

/// The children a summary names, with the servers to reach each through.
///
/// Already ordered: [`children_state`] sorted them when the summary was built,
/// and a remote server is required to have done the same.
///
/// [`children_state`]: Service::children_state
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
