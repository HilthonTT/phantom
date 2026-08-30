//! Reading a room's state at one version.
//!
//! Everything here starts from a shortstatehash and goes through
//! [`load_full_state`], which is the one place the compressed state behind
//! that version is loaded. A single state key is a range query over that set
//! rather than a scan, because [`CompressedStateEvent`] puts the state key
//! first and big-endian.
//!
//! [`load_full_state`]: load_full_state

use std::{borrow::Borrow, ops::Deref, sync::Arc};

use futures::{FutureExt, Stream, StreamExt, TryFutureExt, future::try_join, pin_mut};
use phantom_core::{
    Result, at, err, implement,
    matrix::{PduEvent, StateKey},
    pair_of,
    result::FlatOk,
    stream::{BroadbandExt, IterStream, ReadyExt, TryIgnore},
};
use phantom_database::Deserialized;
use ruma::{
    EventId, OwnedEventId, UserId,
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};
use serde::Deserialize;

use crate::rooms::{
    short::{ShortEventId, ShortStateHash, ShortStateKey},
    state_compressor::{CompressedState, compress_state_event, parse_compressed_state_event},
};

/// Whether the user was a joined member at this version of the state.
#[implement(super::Service)]
#[inline]
pub async fn user_was_joined(&self, shortstatehash: ShortStateHash, user_id: &UserId) -> bool {
    self.user_membership(shortstatehash, user_id).await == MembershipState::Join
}

/// Whether the user was invited or joined at this version of the state.
#[implement(super::Service)]
#[inline]
pub async fn user_was_invited(&self, shortstatehash: ShortStateHash, user_id: &UserId) -> bool {
    let s = self.user_membership(shortstatehash, user_id).await;
    s == MembershipState::Join || s == MembershipState::Invite
}

/// A user's membership at this version of the state.
///
/// Absent membership reads as `Leave`, which is what the spec says a user with
/// no `m.room.member` event in a room has.
#[implement(super::Service)]
pub async fn user_membership(
    &self,
    shortstatehash: ShortStateHash,
    user_id: &UserId,
) -> MembershipState {
    self.state_get_content(
        shortstatehash,
        &StateEventType::RoomMember,
        user_id.as_str(),
    )
    .await
    .map_or(MembershipState::Leave, |c: RoomMemberEventContent| {
        c.membership
    })
}

/// The content of the state event at (`event_type`, `state_key`).
#[implement(super::Service)]
pub async fn state_get_content<T>(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
    state_key: &str,
) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    self.state_get(shortstatehash, event_type, state_key)
        .await
        .and_then(|event| event.get_content())
}

/// Whether the state holds an event at (`event_type`, `state_key`).
#[implement(super::Service)]
pub async fn state_contains(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
    state_key: &str,
) -> bool {
    let Ok(shortstatekey) = self
        .services
        .short
        .get_shortstatekey(event_type, state_key)
        .await
    else {
        return false;
    };

    self.state_contains_shortstatekey(shortstatehash, shortstatekey)
        .await
}

/// Whether the state holds any event of `event_type`, at any state key.
#[implement(super::Service)]
pub async fn state_contains_type(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
) -> bool {
    let state_keys = self.state_keys(shortstatehash, event_type);

    pin_mut!(state_keys);
    state_keys.next().await.is_some()
}

/// [`state_contains`] for a state key whose short id is already in hand.
///
/// [`state_contains`]: Self::state_contains
#[implement(super::Service)]
pub async fn state_contains_shortstatekey(
    &self,
    shortstatehash: ShortStateHash,
    shortstatekey: ShortStateKey,
) -> bool {
    let start = compress_state_event(shortstatekey, 0);
    let end = compress_state_event(shortstatekey, u64::MAX);

    self.load_full_state(shortstatehash)
        .map_ok(|full_state| full_state.range(start..=end).next().copied())
        .await
        .flat_ok()
        .is_some()
}

/// The state event at (`event_type`, `state_key`).
#[implement(super::Service)]
pub async fn state_get(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
    state_key: &str,
) -> Result<PduEvent> {
    self.state_get_id(shortstatehash, event_type, state_key)
        .and_then(|event_id: OwnedEventId| async move {
            self.services.timeline.get_pdu(&event_id).await
        })
        .await
}

/// The event id of the state event at (`event_type`, `state_key`).
#[implement(super::Service)]
pub async fn state_get_id<Id>(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
    state_key: &str,
) -> Result<Id>
where
    Id: for<'de> Deserialize<'de> + Sized + ToOwned,
    <Id as ToOwned>::Owned: Borrow<EventId>,
{
    let shorteventid = self
        .state_get_shortid(shortstatehash, event_type, state_key)
        .await?;

    self.services
        .short
        .get_eventid_from_short(shorteventid)
        .await
}

/// [`state_get_id`] without spelling the event id back out.
///
/// The compressed state is sorted by state key, so the event at one is the
/// first entry in the range that key spans.
///
/// [`state_get_id`]: Self::state_get_id
#[implement(super::Service)]
pub async fn state_get_shortid(
    &self,
    shortstatehash: ShortStateHash,
    event_type: &StateEventType,
    state_key: &str,
) -> Result<ShortEventId> {
    let shortstatekey = self
        .services
        .short
        .get_shortstatekey(event_type, state_key)
        .await?;

    let start = compress_state_event(shortstatekey, 0);
    let end = compress_state_event(shortstatekey, u64::MAX);

    self.load_full_state(shortstatehash)
        .map_ok(|full_state| {
            full_state
                .range(start..=end)
                .next()
                .copied()
                .map(parse_compressed_state_event)
                .map(at!(1))
                .ok_or_else(|| err!(Request(NotFound("Not found in room state"))))
        })
        .await?
}

/// Every state key of `event_type` in the state, with the event at it.
#[implement(super::Service)]
pub fn state_keys_with_ids<'a, Id>(
    &'a self,
    shortstatehash: ShortStateHash,
    event_type: &'a StateEventType,
) -> impl Stream<Item = (StateKey, Id)> + Send + 'a
where
    Id: for<'de> Deserialize<'de> + Send + Sized + ToOwned + 'a,
    <Id as ToOwned>::Owned: Borrow<EventId>,
{
    let state_keys_with_short_ids = self
        .state_keys_with_shortids(shortstatehash, event_type)
        .unzip()
        .map(|(ssks, sids): (Vec<StateKey>, Vec<u64>)| (ssks, sids))
        .shared();

    let state_keys = state_keys_with_short_ids
        .clone()
        .map(at!(0))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    let shorteventids = state_keys_with_short_ids
        .map(at!(1))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    self.services
        .short
        .multi_get_eventid_from_short(shorteventids)
        .zip(state_keys)
        .ready_filter_map(|(eid, sk)| eid.map(move |eid| (sk, eid)).ok())
}

/// [`state_keys_with_ids`] without spelling the event ids back out.
///
/// [`state_keys_with_ids`]: Self::state_keys_with_ids
#[implement(super::Service)]
pub fn state_keys_with_shortids<'a>(
    &'a self,
    shortstatehash: ShortStateHash,
    event_type: &'a StateEventType,
) -> impl Stream<Item = (StateKey, ShortEventId)> + Send + 'a {
    let short_ids = self
        .state_full_shortids(shortstatehash)
        .ignore_err()
        .unzip()
        .map(|(ssks, sids): (Vec<u64>, Vec<u64>)| (ssks, sids))
        .boxed()
        .shared();

    let shortstatekeys = short_ids
        .clone()
        .map(at!(0))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    let shorteventids = short_ids
        .map(at!(1))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    self.services
        .short
        .multi_get_statekey_from_short(shortstatekeys)
        .zip(shorteventids)
        .ready_filter_map(|(res, id)| res.map(|res| (res, id)).ok())
        .ready_filter_map(move |((event_type_, state_key), event_id)| {
            event_type_.eq(event_type).then_some((state_key, event_id))
        })
}

/// Every state key of `event_type` in the state.
#[implement(super::Service)]
pub fn state_keys<'a>(
    &'a self,
    shortstatehash: ShortStateHash,
    event_type: &'a StateEventType,
) -> impl Stream<Item = StateKey> + Send + 'a {
    let short_ids = self
        .state_full_shortids(shortstatehash)
        .ignore_err()
        .map(at!(0));

    self.services
        .short
        .multi_get_statekey_from_short(short_ids)
        .ready_filter_map(Result::ok)
        .ready_filter_map(move |(event_type_, state_key)| {
            event_type_.eq(event_type).then_some(state_key)
        })
}

/// The state events present at `.0` but not at `.1`.
#[implement(super::Service)]
#[inline]
pub fn state_removed(
    &self,
    shortstatehash: pair_of!(ShortStateHash),
) -> impl Stream<Item = (ShortStateKey, ShortEventId)> + Send + '_ {
    self.state_added((shortstatehash.1, shortstatehash.0))
}

/// The state events present at `.1` but not at `.0`.
#[implement(super::Service)]
pub fn state_added(
    &self,
    shortstatehash: pair_of!(ShortStateHash),
) -> impl Stream<Item = (ShortStateKey, ShortEventId)> + Send + '_ {
    let a = self.load_full_state(shortstatehash.0);
    let b = self.load_full_state(shortstatehash.1);

    try_join(a, b)
        .map_ok(|(a, b)| b.difference(&a).copied().collect::<Vec<_>>())
        .map_ok(IterStream::try_stream)
        .try_flatten_stream()
        .ignore_err()
        .map(parse_compressed_state_event)
}

/// The whole state at this version, keyed by type and state key.
#[implement(super::Service)]
pub fn state_full(
    &self,
    shortstatehash: ShortStateHash,
) -> impl Stream<Item = ((StateEventType, StateKey), PduEvent)> + Send + '_ {
    self.state_full_pdus(shortstatehash)
        .ready_filter_map(|pdu| Some(((pdu.kind.to_string().into(), pdu.state_key.clone()?), pdu)))
}

/// The whole state at this version, as bare PDUs.
#[implement(super::Service)]
pub fn state_full_pdus(
    &self,
    shortstatehash: ShortStateHash,
) -> impl Stream<Item = PduEvent> + Send + '_ {
    let short_ids = self
        .state_full_shortids(shortstatehash)
        .ignore_err()
        .map(at!(1));

    self.services
        .short
        .multi_get_eventid_from_short(short_ids)
        .ready_filter_map(Result::ok)
        .broad_filter_map(move |event_id: OwnedEventId| async move {
            self.services.timeline.get_pdu(&event_id).await.ok()
        })
}

/// The whole state at this version, as short state key to event id.
#[implement(super::Service)]
pub fn state_full_ids<'a, Id>(
    &'a self,
    shortstatehash: ShortStateHash,
) -> impl Stream<Item = (ShortStateKey, Id)> + Send + 'a
where
    Id: for<'de> Deserialize<'de> + Send + Sized + ToOwned + 'a,
    <Id as ToOwned>::Owned: Borrow<EventId>,
{
    let shortids = self
        .state_full_shortids(shortstatehash)
        .ignore_err()
        .unzip()
        .shared();

    let shortstatekeys = shortids
        .clone()
        .map(at!(0))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    let shorteventids = shortids
        .map(at!(1))
        .map(Vec::into_iter)
        .map(IterStream::stream)
        .flatten_stream();

    self.services
        .short
        .multi_get_eventid_from_short(shorteventids)
        .zip(shortstatekeys)
        .ready_filter_map(|(event_id, shortstatekey)| Some((shortstatekey, event_id.ok()?)))
}

/// The whole state at this version, as pairs of short ids.
///
/// This is the bottom of the module: everything above it is these pairs with
/// one half or the other spelled back out.
#[implement(super::Service)]
pub fn state_full_shortids(
    &self,
    shortstatehash: ShortStateHash,
) -> impl Stream<Item = Result<(ShortStateKey, ShortEventId)>> + Send + '_ {
    self.load_full_state(shortstatehash)
        .map_ok(|full_state| {
            full_state
                .deref()
                .iter()
                .copied()
                .map(parse_compressed_state_event)
                .collect()
        })
        .map_ok(Vec::into_iter)
        .map_ok(IterStream::try_stream)
        .try_flatten_stream()
        .boxed()
}

/// The compressed state at this version.
///
/// The top layer of the stack carries the full state, the layers below it
/// being the diffs it was rebuilt from.
#[implement(super::Service)]
#[tracing::instrument(name = "load", level = "debug", skip(self))]
async fn load_full_state(&self, shortstatehash: ShortStateHash) -> Result<Arc<CompressedState>> {
    self.services
        .state_compressor
        .load_shortstatehash_info(shortstatehash)
        .map_err(|e| err!(Database("Missing state IDs: {e}")))
        .map_ok(|vec| vec.last().expect("at least one layer").full_state.clone())
        .await
}

/// The version of the room's state that this event was accepted against.
#[implement(super::Service)]
pub async fn pdu_shortstatehash(&self, event_id: &EventId) -> Result<ShortStateHash> {
    const BUFSIZE: usize = size_of::<ShortEventId>();

    self.services
        .short
        .get_shorteventid(event_id)
        .and_then(|shorteventid| {
            self.db
                .shorteventid_shortstatehash
                .aqry::<BUFSIZE, _>(&shorteventid)
        })
        .await
        .deserialized()
}
