//! Which version of its state a room is currently at, and moving it forward.
//!
//! [`state_compressor`] owns how a version is stored and [`state_accessor`]
//! owns reading one back. What is here is the pointer from a room to its
//! current version, the forward extremities that say where the timeline ends,
//! and the two ways a new version comes about: [`append_to_state`] as one
//! event lands, and [`force_state`] when state resolution has decided the
//! whole thing at once.
//!
//! [`mutex`](Service::mutex) is held across any of that. A room's state is
//! read, changed and written back, so two events landing at once would
//! otherwise each write a version derived from the state before the other.
//!
//! [`append_to_state`]: Service::append_to_state
//! [`force_state`]: Service::force_state
//! [`state_accessor`]: crate::rooms::state_accessor
//! [`state_compressor`]: crate::rooms::state_compressor

use std::{collections::HashMap, fmt::Write, iter::once, sync::Arc};

use async_trait::async_trait;
use futures::{
    FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt, future::join_all, pin_mut,
};
use phantom_core::{
    Result, err, hash,
    matrix::{
        PduEvent,
        state_res::{self, StateMap},
    },
    result::FlatOk,
    stream::{BroadbandExt, IterStream, ReadyExt, TryIgnore},
    sync::{MutexMap, MutexMapGuard},
};
use phantom_database::{Deserialized, Ignore, Interfix, Map, serialize_to_vec};
use ruma::{
    EventId, OwnedEventId, OwnedRoomId, RoomId, RoomVersionId, UserId,
    events::{
        AnyStrippedStateEvent, StateEventType, TimelineEventType,
        room::{create::RoomCreateEventContent, member::RoomMemberEventContent},
    },
    serde::Raw,
};

use crate::{
    Dep, rooms,
    rooms::{
        short::{ShortEventId, ShortStateHash},
        state_compressor::{CompressedState, compress_state_event, parse_compressed_state_event},
    },
    server_state,
};

pub struct Service {
    /// Held for the length of any read-modify-write of a room's state.
    ///
    /// Per room rather than global: two rooms have nothing to serialize
    /// against each other.
    pub mutex: RoomMutexMap,
    services: Services,
    db: Data,
}

struct Services {
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    state_compressor: Dep<rooms::state_compressor::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

struct Data {
    shorteventid_shortstatehash: Arc<Map>,
    roomid_shortstatehash: Arc<Map>,
    roomid_pduleaves: Arc<Map>,
}

type RoomMutexMap = MutexMap<OwnedRoomId, ()>;
pub type RoomMutexGuard = MutexMapGuard<OwnedRoomId, ()>;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            mutex: RoomMutexMap::new(),
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                state_compressor: args
                    .depend::<rooms::state_compressor::Service>("rooms::state_compressor"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
            db: Data {
                shorteventid_shortstatehash: args.db["shorteventid_shortstatehash"].clone(),
                roomid_shortstatehash: args.db["roomid_shortstatehash"].clone(),
                roomid_pduleaves: args.db["roomid_pduleaves"].clone(),
            },
        }))
    }

    async fn memory_usage(&self, out: &mut (dyn Write + Send)) -> Result {
        writeln!(out, "state_mutex: {}", self.mutex.len())?;

        Ok(())
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Moves the room to a state decided wholesale, bringing the membership
    /// indexes along with it.
    ///
    /// This is what state resolution's answer is applied through, as against
    /// [`append_to_state`], which advances the state one event at a time.
    ///
    /// [`append_to_state`]: Self::append_to_state
    pub async fn force_state(
        &self,
        room_id: &RoomId,
        shortstatehash: ShortStateHash,
        statediffnew: Arc<CompressedState>,
        _statediffremoved: Arc<CompressedState>,
        state_lock: &RoomMutexGuard,
    ) -> Result {
        let event_ids = statediffnew
            .iter()
            .stream()
            .map(|&new| parse_compressed_state_event(new).1)
            .then(|shorteventid| {
                self.services
                    .short
                    .get_eventid_from_short::<OwnedEventId>(shorteventid)
            })
            .ignore_err();

        // Only membership needs following through: it is the state the
        // indexes in `state_cache` duplicate, so it is the state that goes
        // stale if the new version is written without them.
        pin_mut!(event_ids);
        while let Some(event_id) = event_ids.next().await {
            let Ok(pdu) = self.services.timeline.get_pdu(&event_id).await else {
                continue;
            };

            if pdu.kind != TimelineEventType::RoomMember {
                continue;
            }

            let Some(user_id) = pdu.state_key.as_deref().map(UserId::parse).flat_ok() else {
                continue;
            };

            let Ok(membership_event) = pdu.get_content::<RoomMemberEventContent>() else {
                continue;
            };

            self.services
                .state_cache
                .update_membership(
                    room_id,
                    &user_id,
                    membership_event,
                    &pdu.sender,
                    None,
                    None,
                    false,
                )
                .await?;
        }

        self.services.state_cache.update_joined_count(room_id).await;

        self.set_room_state(room_id, shortstatehash, state_lock);

        Ok(())
    }

    /// Records the state an event was accepted against, without making it the
    /// room's current state.
    ///
    /// The version is derived from the state itself, so an event accepted
    /// against a state this server already knows reuses that version.
    #[tracing::instrument(skip(self, state_ids_compressed), level = "debug")]
    pub async fn set_event_state(
        &self,
        event_id: &EventId,
        room_id: &RoomId,
        state_ids_compressed: Arc<CompressedState>,
    ) -> Result<ShortStateHash> {
        const KEY_LEN: usize = size_of::<ShortEventId>();
        const VAL_LEN: usize = size_of::<ShortStateHash>();

        let shorteventid = self
            .services
            .short
            .get_or_create_shorteventid(event_id)
            .await;

        let previous_shortstatehash = self.get_room_shortstatehash(room_id).await;

        let state_hash = hash::sha256::delimited(state_ids_compressed.iter().map(|s| &s[..]));

        let (shortstatehash, already_existed) = self
            .services
            .short
            .get_or_create_shortstatehash(&state_hash)
            .await;

        if !already_existed {
            let states_parents = match previous_shortstatehash {
                Ok(p) => {
                    self.services
                        .state_compressor
                        .load_shortstatehash_info(p)
                        .await?
                }
                _ => Vec::new(),
            };

            let (statediffnew, statediffremoved) =
                if let Some(parent_stateinfo) = states_parents.last() {
                    let statediffnew: CompressedState = state_ids_compressed
                        .difference(&parent_stateinfo.full_state)
                        .copied()
                        .collect();

                    let statediffremoved: CompressedState = parent_stateinfo
                        .full_state
                        .difference(&state_ids_compressed)
                        .copied()
                        .collect();

                    (Arc::new(statediffnew), Arc::new(statediffremoved))
                } else {
                    (state_ids_compressed, Arc::new(CompressedState::new()))
                };

            self.services.state_compressor.save_state_from_diff(
                shortstatehash,
                statediffnew,
                statediffremoved,
                // Deliberately large: nothing will be layered on top of a
                // state recorded for one event, so there is no sibling for
                // this diff to be judged against.
                1_000_000,
                states_parents,
            )?;
        }

        self.db
            .shorteventid_shortstatehash
            .aput::<KEY_LEN, VAL_LEN, _, _>(shorteventid, shortstatehash)
            .ok();

        Ok(shortstatehash)
    }

    /// Advances the room's state by one event, returning the version that
    /// results.
    ///
    /// The caller is expected to make it the room's current state with
    /// [`set_room_state`] once the event itself has been written.
    ///
    /// [`set_room_state`]: Self::set_room_state
    #[tracing::instrument(skip(self, new_pdu), level = "debug")]
    pub async fn append_to_state(&self, new_pdu: &PduEvent) -> Result<ShortStateHash> {
        const BUFSIZE: usize = size_of::<u64>();

        let shorteventid = self
            .services
            .short
            .get_or_create_shorteventid(&new_pdu.event_id)
            .await;

        let previous_shortstatehash = self.get_room_shortstatehash(&new_pdu.room_id).await;

        // The state the event was accepted against is the state *before* it,
        // which is the room's current version at this point.
        if let Ok(p) = previous_shortstatehash {
            self.db
                .shorteventid_shortstatehash
                .aput::<BUFSIZE, BUFSIZE, _, _>(shorteventid, p)
                .ok();
        }

        let Some(state_key) = &new_pdu.state_key else {
            // Not a state event, so the state is unchanged.
            return previous_shortstatehash
                .map_err(|e| err!(Database("first event in room must be a state event: {e}")));
        };

        let states_parents = match previous_shortstatehash {
            Ok(p) => {
                self.services
                    .state_compressor
                    .load_shortstatehash_info(p)
                    .await?
            }
            _ => Vec::new(),
        };

        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&new_pdu.kind.to_string().into(), state_key)
            .await;

        let new = self
            .services
            .state_compressor
            .compress_state_event(shortstatekey, &new_pdu.event_id)
            .await;

        // The compressed form sorts by state key, so what this event replaces
        // is the first entry in the range that key spans. A range query rather
        // than a scan: this runs for every state event, and the alternative
        // walks the room's entire state each time.
        let start = compress_state_event(shortstatekey, 0);
        let end = compress_state_event(shortstatekey, ShortEventId::MAX);
        let replaces = states_parents
            .last()
            .and_then(|info| info.full_state.range(start..=end).next());

        if Some(&new) == replaces {
            return previous_shortstatehash.map_err(|e| {
                err!(Database(
                    "state event replaces itself in an empty room: {e}"
                ))
            });
        }

        // TODO: derive the version from the state, as `set_event_state` does,
        // so that two servers reaching the same state agree on its short id.
        let shortstatehash = self.services.server_state.next_count()?;

        let mut statediffnew = CompressedState::new();
        statediffnew.insert(new);

        let mut statediffremoved = CompressedState::new();
        if let Some(replaces) = replaces {
            statediffremoved.insert(*replaces);
        }

        self.services.state_compressor.save_state_from_diff(
            shortstatehash,
            Arc::new(statediffnew),
            Arc::new(statediffremoved),
            2,
            states_parents,
        )?;

        Ok(shortstatehash)
    }

    /// The handful of state events a client is shown for a room it has been
    /// invited to but cannot yet read.
    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn summary_stripped(&self, event: &PduEvent) -> Vec<Raw<AnyStrippedStateEvent>> {
        let cells = [
            (&StateEventType::RoomCreate, ""),
            (&StateEventType::RoomJoinRules, ""),
            (&StateEventType::RoomCanonicalAlias, ""),
            (&StateEventType::RoomName, ""),
            (&StateEventType::RoomAvatar, ""),
            // So the invitee can see who invited them.
            (&StateEventType::RoomMember, event.sender.as_str()),
            (&StateEventType::RoomEncryption, ""),
            (&StateEventType::RoomTopic, ""),
        ];

        let fetches = cells.iter().map(|(event_type, state_key)| {
            self.services
                .state_accessor
                .room_state_get(&event.room_id, event_type, state_key)
        });

        join_all(fetches)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .map(PduEvent::into_stripped_state_event)
            .chain(once(event.to_stripped_state_event()))
            .collect()
    }

    /// Makes `shortstatehash` the room's current state.
    ///
    /// Takes the guard to make the ordering explicit rather than to use it:
    /// deciding a new version and installing it have to be one step.
    #[tracing::instrument(skip(self, _mutex_lock), level = "debug")]
    pub fn set_room_state(
        &self,
        room_id: &RoomId,
        shortstatehash: ShortStateHash,
        _mutex_lock: &RoomMutexGuard,
    ) {
        const BUFSIZE: usize = size_of::<u64>();

        self.db
            .roomid_shortstatehash
            .raw_aput::<BUFSIZE, _, _>(room_id, shortstatehash)
            .ok();
    }

    /// The room's version, from `m.room.create`.
    #[tracing::instrument(skip(self), level = "debug")]
    pub async fn get_room_version(&self, room_id: &RoomId) -> Result<RoomVersionId> {
        self.services
            .state_accessor
            .room_state_get_content(room_id, &StateEventType::RoomCreate, "")
            .await
            .map(|content: RoomCreateEventContent| content.room_version)
            .map_err(|e| err!(Request(NotFound("No create event found: {e:?}"))))
    }

    /// The version of the state the room is currently at.
    pub async fn get_room_shortstatehash(&self, room_id: &RoomId) -> Result<ShortStateHash> {
        self.db
            .roomid_shortstatehash
            .get(room_id)
            .await
            .deserialized()
    }

    /// The events at the end of the room's timeline, which are what the next
    /// event will name as its parents.
    pub fn get_forward_extremities<'a>(
        &'a self,
        room_id: &'a RoomId,
    ) -> impl Stream<Item = &'a EventId> + Send + 'a {
        let prefix = (room_id, Interfix);

        self.db
            .roomid_pduleaves
            .keys_prefix(&prefix)
            .map_ok(|(_, event_id): (Ignore, &str)| {
                <&EventId>::try_from(event_id).expect("valid event id in db")
            })
            .ignore_err()
    }

    /// Replaces the room's forward extremities wholesale.
    pub async fn set_forward_extremities<'a, I>(
        &'a self,
        room_id: &'a RoomId,
        event_ids: I,
        _state_lock: &'a RoomMutexGuard,
    ) where
        I: Iterator<Item = &'a EventId> + Send + 'a,
    {
        let prefix = serialize_to_vec((room_id, Interfix)).expect("failed to serialize prefix");

        self.db
            .roomid_pduleaves
            .raw_keys_prefix(&prefix)
            .ignore_err()
            .ready_for_each(|key| {
                self.db.roomid_pduleaves.remove(key).ok();
            })
            .await;

        for event_id in event_ids {
            self.db
                .roomid_pduleaves
                .put_raw((room_id, event_id), event_id)
                .ok();
        }
    }

    /// The events authorizing an event that has not been built yet.
    ///
    /// Which state events those are is decided by the event's own type,
    /// sender and content; this looks each one up in the room's current state
    /// and returns the ones that exist.
    #[tracing::instrument(skip(self, content), level = "debug")]
    pub async fn get_auth_events(
        &self,
        room_id: &RoomId,
        kind: &TimelineEventType,
        sender: &UserId,
        state_key: Option<&str>,
        content: &serde_json::value::RawValue,
    ) -> Result<StateMap<PduEvent>> {
        let Ok(shortstatehash) = self.get_room_shortstatehash(room_id).await else {
            // A room with no state is a room being created, whose first event
            // has nothing to be authorized against.
            return Ok(HashMap::new());
        };

        let auth_types = state_res::auth_types_for_event(kind, sender, state_key, content)?;

        // Matching on short ids rather than on (type, state key) pairs: the
        // state is walked once, and each entry is one integer comparison.
        let sauthevents: HashMap<_, _> = auth_types
            .iter()
            .stream()
            .broad_filter_map(|(event_type, state_key)| {
                self.services
                    .short
                    .get_shortstatekey(event_type, state_key)
                    .map_ok(move |ssk| (ssk, (event_type, state_key)))
                    .map(Result::ok)
            })
            .collect()
            .await;

        let (state_keys, event_ids): (Vec<_>, Vec<_>) = self
            .services
            .state_accessor
            .state_full_shortids(shortstatehash)
            .ready_filter_map(Result::ok)
            .ready_filter_map(|(shortstatekey, shorteventid)| {
                sauthevents
                    .get(&shortstatekey)
                    .map(|(ty, sk)| ((ty, sk), shorteventid))
            })
            .unzip()
            .await;

        self.services
            .short
            .multi_get_eventid_from_short(event_ids.into_iter().stream())
            .zip(state_keys.into_iter().stream())
            .ready_filter_map(|(event_id, (ty, sk))| Some(((ty, sk), event_id.ok()?)))
            .broad_filter_map(|((ty, sk), event_id): (_, OwnedEventId)| async move {
                self.services
                    .timeline
                    .get_pdu(&event_id)
                    .await
                    .map(move |pdu| (((*ty).clone(), (*sk).clone()), pdu))
                    .ok()
            })
            .collect()
            .map(Ok)
            .await
    }
}
