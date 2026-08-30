//! A room's state, stored as a stack of diffs rather than a copy per version.
//!
//! Every event that changes state produces a new version of the whole state,
//! and a busy room produces thousands. Storing each in full would store the
//! same membership event over and over, so a version is stored as what it
//! added and removed relative to a parent version, and the full state is
//! rebuilt by walking down to the bottom layer and applying each diff on the
//! way back up.
//!
//! The layers are rebalanced as they grow: a diff that has become large
//! relative to its parent is merged into that parent instead of stacked on
//! top of it, which keeps the walk short. [`save_state_from_diff`] is where
//! that decision is made.
//!
//! State is held here as [`CompressedStateEvent`] — a shortstatekey and a
//! shorteventid packed into sixteen bytes — because the whole point is to fit
//! a room's entire state in memory, and because a `BTreeSet` of those sorts by
//! state key, which is what makes "the event at this state key" a range query
//! rather than a scan.
//!
//! [`save_state_from_diff`]: Service::save_state_from_diff

use std::{
    collections::{BTreeSet, HashMap},
    fmt::{Debug, Write},
    sync::{Arc, Mutex},
};

use arrayvec::ArrayVec;
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use lru_cache::LruCache;
use phantom_core::{
    Result, at, bytes,
    bytes::{u64_from_bytes, u64_from_u8},
    checked, err, expected, hash, implement,
    math::usize_from_f64,
    stream::IterStream,
};
use phantom_database::Map;
use ruma::{EventId, RoomId};

use crate::{
    Dep, rooms,
    rooms::short::{ShortEventId, ShortId, ShortStateHash, ShortStateKey},
};

pub struct Service {
    /// The layer stack behind a shortstatehash, keyed by that hash.
    ///
    /// Public because the admin surface reports on it; it is not a way in for
    /// other services, which go through [`load_shortstatehash_info`].
    ///
    /// [`load_shortstatehash_info`]: Service::load_shortstatehash_info
    pub stateinfo_cache: Mutex<StateInfoLruCache>,
    db: Data,
    services: Services,
}

struct Services {
    short: Dep<rooms::short::Service>,
    state: Dep<rooms::state::Service>,
}

struct Data {
    shortstatehash_statediff: Arc<Map>,
}

/// One version of a room's state, as stored: what it changed and where it
/// changed from.
#[derive(Clone)]
struct StateDiff {
    parent: Option<ShortStateHash>,
    added: Arc<CompressedState>,
    removed: Arc<CompressedState>,
}

/// One layer of the stack, with the full state as of that layer already
/// applied.
#[derive(Clone, Default)]
pub struct ShortStateInfo {
    pub shortstatehash: ShortStateHash,
    pub full_state: Arc<CompressedState>,
    pub added: Arc<CompressedState>,
    pub removed: Arc<CompressedState>,
}

/// What a state change amounted to, as returned by [`Service::save_state`].
#[derive(Clone, Default)]
pub struct HashSetCompressStateEvent {
    pub shortstatehash: ShortStateHash,
    pub added: Arc<CompressedState>,
    pub removed: Arc<CompressedState>,
}

pub type StateInfoLruCache = LruCache<ShortStateHash, ShortStateInfoVec>;
type ShortStateInfoVec = Vec<ShortStateInfo>;
type ParentStatesVec = Vec<ShortStateInfo>;

/// A room's state at one version, sorted by state key.
pub type CompressedState = BTreeSet<CompressedStateEvent>;

/// A shortstatekey and a shorteventid, big-endian, one after the other.
///
/// Big-endian and in that order so that the natural byte ordering sorts by
/// state key, which is what lets [`state_get_shortid`] find the event at a
/// state key with a range query.
///
/// [`state_get_shortid`]: crate::rooms::state_accessor::Service::state_get_shortid
pub type CompressedStateEvent = [u8; 2 * size_of::<ShortId>()];

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;
        let cache_capacity =
            f64::from(config.stateinfo_cache_capacity) * config.cache_capacity_modifier;

        Ok(Arc::new(Self {
            stateinfo_cache: LruCache::new(usize_from_f64(cache_capacity)?).into(),
            db: Data {
                shortstatehash_statediff: args.db["shortstatehash_statediff"].clone(),
            },
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
            },
        }))
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        // The same `Arc<CompressedState>` is shared by the layers of one stack
        // and across stacks, so the sets are counted by address: adding up
        // every layer's length would report the same allocation many times.
        let (cache_len, ents) = {
            let cache = self.stateinfo_cache.lock().expect("locked");
            let ents = cache.iter().map(at!(1)).flat_map(|vec| vec.iter()).fold(
                HashMap::new(),
                |mut ents, ssi| {
                    for cs in &[&ssi.added, &ssi.removed, &ssi.full_state] {
                        ents.insert(Arc::as_ptr(cs), compressed_state_size(cs));
                    }

                    ents
                },
            );

            (cache.len(), ents)
        };

        let ents_len = ents.len();
        let bytes = ents.values().copied().fold(0_usize, usize::saturating_add);

        writeln!(
            out,
            "stateinfo_cache: {cache_len} {ents_len} ({})",
            bytes::pretty(bytes)
        )?;

        Ok(())
    }

    async fn clear_cache(&self) {
        self.stateinfo_cache.lock().expect("locked").clear();
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// The layer stack for `shortstatehash`, bottom layer first.
///
/// The last frame is the one asked for, and its `full_state` is the room's
/// whole state at that version.
#[implement(Service)]
#[tracing::instrument(name = "load", level = "debug", skip(self))]
pub async fn load_shortstatehash_info(
    &self,
    shortstatehash: ShortStateHash,
) -> Result<ShortStateInfoVec> {
    if let Some(r) = self
        .stateinfo_cache
        .lock()
        .expect("locked")
        .get_mut(&shortstatehash)
    {
        return Ok(r.clone());
    }

    let stack = self.new_shortstatehash_info(shortstatehash).await?;

    self.stateinfo_cache
        .lock()
        .expect("locked")
        .insert(shortstatehash, stack.clone());

    Ok(stack)
}

/// [`load_shortstatehash_info`] for a stack that was not cached: walk to the
/// bottom layer and apply each diff on the way back up.
///
/// [`load_shortstatehash_info`]: Service::load_shortstatehash_info
#[implement(Service)]
async fn new_shortstatehash_info(
    &self,
    shortstatehash: ShortStateHash,
) -> Result<ShortStateInfoVec> {
    let StateDiff {
        parent,
        added,
        removed,
    } = self.get_statediff(shortstatehash).await?;

    let Some(parent) = parent else {
        return Ok(vec![ShortStateInfo {
            shortstatehash,
            full_state: added.clone(),
            added,
            removed,
        }]);
    };

    // Boxed because this recurses once per layer, and the layers are only
    // bounded by the rebalancing in `save_state_from_diff`.
    let mut stack = Box::pin(self.load_shortstatehash_info(parent)).await?;
    let top = stack.last().expect("at least one frame");

    let mut full_state = (*top.full_state).clone();
    full_state.extend(added.iter().copied());

    let removed = (*removed).clone();
    for r in &removed {
        full_state.remove(r);
    }

    stack.push(ShortStateInfo {
        shortstatehash,
        added,
        removed: Arc::new(removed),
        full_state: Arc::new(full_state),
    });

    Ok(stack)
}

/// Compresses a state map, creating a short id for any event that lacks one.
#[implement(Service)]
pub fn compress_state_events<'a, I>(
    &'a self,
    state: I,
) -> impl Stream<Item = CompressedStateEvent> + Send + 'a
where
    I: Iterator<Item = (&'a ShortStateKey, &'a EventId)> + Clone + Debug + Send + 'a,
{
    let event_ids = state.clone().map(at!(1));

    let short_event_ids = self
        .services
        .short
        .multi_get_or_create_shorteventid(event_ids);

    state
        .stream()
        .map(at!(0))
        .zip(short_event_ids)
        .map(|(shortstatekey, shorteventid)| compress_state_event(*shortstatekey, shorteventid))
}

/// [`compress_state_events`] for one event.
///
/// [`compress_state_events`]: Service::compress_state_events
#[implement(Service)]
pub async fn compress_state_event(
    &self,
    shortstatekey: ShortStateKey,
    event_id: &EventId,
) -> CompressedStateEvent {
    let shorteventid = self
        .services
        .short
        .get_or_create_shorteventid(event_id)
        .await;

    compress_state_event(shortstatekey, shorteventid)
}

/// Stores `shortstatehash` as a diff, choosing which layer to put it on.
///
/// Layer 0 holds a full state and each layer above it holds a diff against
/// the one below. A diff that has grown large relative to its parent is
/// merged into that parent rather than stacked on top of it, which keeps the
/// stack shallow; merging can in turn make the parent too large, so this
/// recurses.
///
/// * `shortstatehash` — the version being stored
/// * `statediffnew` — what this version adds to the layer below
/// * `statediffremoved` — what it removes from the layer below
/// * `diff_to_sibling` — roughly how much a diff on this layer grows each time,
///   which is what "large relative to its parent" is measured against
/// * `parent_states` — the stack below, as [`load_shortstatehash_info`] returns
///   it
///
/// [`load_shortstatehash_info`]: Service::load_shortstatehash_info
#[implement(Service)]
pub fn save_state_from_diff(
    &self,
    shortstatehash: ShortStateHash,
    statediffnew: Arc<CompressedState>,
    statediffremoved: Arc<CompressedState>,
    diff_to_sibling: usize,
    mut parent_states: ParentStatesVec,
) -> Result {
    let statediffnew_len = statediffnew.len();
    let statediffremoved_len = statediffremoved.len();
    let diffsum = checked!(statediffnew_len + statediffremoved_len)?;

    if parent_states.len() > 3 {
        // Too many layers: fold this diff into the one below and try again
        // one layer down.
        let parent = parent_states.pop().expect("parent must have a state");
        let (parent_new, parent_removed) =
            merge_into_parent(&parent, &statediffnew, &statediffremoved);

        return self.save_state_from_diff(
            shortstatehash,
            Arc::new(parent_new),
            Arc::new(parent_removed),
            diffsum,
            parent_states,
        );
    }

    if parent_states.is_empty() {
        // Nothing below, so this diff *is* the full state.
        self.save_statediff(
            shortstatehash,
            &StateDiff {
                parent: None,
                added: statediffnew,
                removed: statediffremoved,
            },
        );

        return Ok(());
    }

    let parent = parent_states.pop().expect("parent must have a state");
    let parent_added_len = parent.added.len();
    let parent_removed_len = parent.removed.len();
    let parent_diff = checked!(parent_added_len + parent_removed_len)?;

    if checked!(diffsum * diffsum)? >= checked!(2 * diff_to_sibling * parent_diff)? {
        // The diff has outgrown the layer it would sit on: replace that layer
        // instead of adding to the stack.
        let (parent_new, parent_removed) =
            merge_into_parent(&parent, &statediffnew, &statediffremoved);

        self.save_state_from_diff(
            shortstatehash,
            Arc::new(parent_new),
            Arc::new(parent_removed),
            diffsum,
            parent_states,
        )?;
    } else {
        self.save_statediff(
            shortstatehash,
            &StateDiff {
                parent: Some(parent.shortstatehash),
                added: statediffnew,
                removed: statediffremoved,
            },
        );
    }

    Ok(())
}

/// Records a room's new state, and reports what changed.
///
/// The version is derived from the state itself, so a state this server has
/// seen before is recognized rather than stored twice.
#[implement(Service)]
#[tracing::instrument(skip(self, new_state_ids_compressed), level = "debug")]
pub async fn save_state(
    &self,
    room_id: &RoomId,
    new_state_ids_compressed: Arc<CompressedState>,
) -> Result<HashSetCompressStateEvent> {
    let previous_shortstatehash = self
        .services
        .state
        .get_room_shortstatehash(room_id)
        .await
        .ok();

    let state_hash =
        hash::sha256::delimited(new_state_ids_compressed.iter().map(|bytes| &bytes[..]));

    let (new_shortstatehash, already_existed) = self
        .services
        .short
        .get_or_create_shortstatehash(&state_hash)
        .await;

    if Some(new_shortstatehash) == previous_shortstatehash {
        return Ok(HashSetCompressStateEvent {
            shortstatehash: new_shortstatehash,
            ..Default::default()
        });
    }

    let states_parents = if let Some(p) = previous_shortstatehash {
        self.load_shortstatehash_info(p).await.unwrap_or_default()
    } else {
        ShortStateInfoVec::new()
    };

    let (statediffnew, statediffremoved) = if let Some(parent_stateinfo) = states_parents.last() {
        let statediffnew: CompressedState = new_state_ids_compressed
            .difference(&parent_stateinfo.full_state)
            .copied()
            .collect();

        let statediffremoved: CompressedState = parent_stateinfo
            .full_state
            .difference(&new_state_ids_compressed)
            .copied()
            .collect();

        (Arc::new(statediffnew), Arc::new(statediffremoved))
    } else {
        (new_state_ids_compressed, Arc::new(CompressedState::new()))
    };

    if !already_existed {
        self.save_state_from_diff(
            new_shortstatehash,
            statediffnew.clone(),
            statediffremoved.clone(),
            2, // a state change is two event changes on average
            states_parents,
        )?;
    }

    Ok(HashSetCompressStateEvent {
        shortstatehash: new_shortstatehash,
        added: statediffnew,
        removed: statediffremoved,
    })
}

/// Reads one layer back out of the column.
///
/// The stored form is the parent's hash, then the added events, then — only
/// if anything was removed — a zero word and the removed events. Zero is
/// usable as the separator because it is never a valid shortstatekey, the
/// counter every short id comes from starting at one.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug", name = "get")]
async fn get_statediff(&self, shortstatehash: ShortStateHash) -> Result<StateDiff> {
    const BUFSIZE: usize = size_of::<ShortStateHash>();
    const STRIDE: usize = size_of::<ShortStateHash>();

    let value = self
        .db
        .shortstatehash_statediff
        .aqry::<BUFSIZE, _>(&shortstatehash)
        .await
        .map_err(|e| {
            err!(Database(
                "Failed to find StateDiff from short {shortstatehash:?}: {e}"
            ))
        })?;

    let parent = u64_from_bytes(&value[0..size_of::<u64>()])
        .ok()
        .take_if(|parent| *parent != 0);

    debug_assert!(value.len() % STRIDE == 0, "value not aligned to stride");

    let mut add_mode = true;
    let mut added = CompressedState::new();
    let mut removed = CompressedState::new();

    let mut i = STRIDE;
    while let Some(v) = value.get(i..expected!(i + 2 * STRIDE)) {
        if add_mode && v.starts_with(&0_u64.to_be_bytes()) {
            add_mode = false;
            i = expected!(i + STRIDE);
            continue;
        }
        if add_mode {
            added.insert(v.try_into()?);
        } else {
            removed.insert(v.try_into()?);
        }
        i = expected!(i + 2 * STRIDE);
    }

    Ok(StateDiff {
        parent,
        added: Arc::new(added),
        removed: Arc::new(removed),
    })
}

/// The write half of [`get_statediff`](Service::get_statediff).
#[implement(Service)]
fn save_statediff(&self, shortstatehash: ShortStateHash, diff: &StateDiff) {
    // In bytes, not entries: the parent word, then one compressed event per
    // addition, then the separator word and one per removal.
    const WORD: usize = size_of::<ShortStateHash>();
    const ENTRY: usize = size_of::<CompressedStateEvent>();

    let separator = usize::from(!diff.removed.is_empty()).saturating_mul(WORD);
    let entries = diff
        .added
        .len()
        .saturating_add(diff.removed.len())
        .saturating_mul(ENTRY);

    let mut value =
        Vec::<u8>::with_capacity(WORD.saturating_add(separator).saturating_add(entries));

    let parent = diff.parent.unwrap_or(0_u64);
    value.extend_from_slice(&parent.to_be_bytes());

    for new in diff.added.iter() {
        value.extend_from_slice(&new[..]);
    }

    if !diff.removed.is_empty() {
        value.extend_from_slice(&0_u64.to_be_bytes());
        for removed in diff.removed.iter() {
            value.extend_from_slice(&removed[..]);
        }
    }

    self.db
        .shortstatehash_statediff
        .insert(&shortstatehash.to_be_bytes(), &value)
        .ok();
}

/// Applies a child's diff onto its parent's, yielding the diff the parent
/// would have had if the child had never been a layer of its own.
///
/// A change that the child undoes is dropped rather than recorded twice: an
/// event the parent added and the child removed is simply not in the result.
fn merge_into_parent(
    parent: &ShortStateInfo,
    statediffnew: &CompressedState,
    statediffremoved: &CompressedState,
) -> (CompressedState, CompressedState) {
    let mut parent_new = (*parent.added).clone();
    let mut parent_removed = (*parent.removed).clone();

    for removed in statediffremoved {
        if !parent_new.remove(removed) {
            // The parent did not add it, so the removal is the parent's now.
            parent_removed.insert(*removed);
        }
        // Else the parent added it and the child took it away again, which
        // together is no change at all.
    }

    for new in statediffnew {
        if !parent_removed.remove(new) {
            // The parent did not touch it, so the addition is the parent's now.
            parent_new.insert(*new);
        }
        // Else the parent removed it and the child put it back.
    }

    (parent_new, parent_removed)
}

/// Packs a state key and an event id into one [`CompressedStateEvent`].
#[inline]
#[must_use]
pub(crate) fn compress_state_event(
    shortstatekey: ShortStateKey,
    shorteventid: ShortEventId,
) -> CompressedStateEvent {
    const SIZE: usize = size_of::<CompressedStateEvent>();

    let mut v = ArrayVec::<u8, SIZE>::new();
    v.extend(shortstatekey.to_be_bytes());
    v.extend(shorteventid.to_be_bytes());
    v.as_ref()
        .try_into()
        .expect("failed to create CompressedStateEvent")
}

/// The inverse of [`compress_state_event`].
#[inline]
#[must_use]
pub(crate) fn parse_compressed_state_event(
    compressed_event: CompressedStateEvent,
) -> (ShortStateKey, ShortEventId) {
    let shortstatekey = u64_from_u8(&compressed_event[0..size_of::<ShortStateKey>()]);
    let shorteventid = u64_from_u8(&compressed_event[size_of::<ShortStateKey>()..]);

    (shortstatekey, shorteventid)
}

#[inline]
fn compressed_state_size(compressed_state: &CompressedState) -> usize {
    compressed_state
        .len()
        .checked_mul(size_of::<CompressedStateEvent>())
        .expect("CompressedState size overflow")
}
