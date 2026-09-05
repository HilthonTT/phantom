//! The PDUs of a room, in order.
//!
//! The read path is here — looking a PDU up by event id or by the id it is
//! stored under, and streaming a room's PDUs in either direction — and the
//! write path is in [`append`], which is where an event becomes part of a
//! room rather than merely being known about.
//!
//! What is still parked in `pending_write_path.rs`, which is not a module of
//! this one, is the part that builds an event of this server's own
//! (`create_hash_and_sign_event`, `build_and_append_pdu`) and backfill. Those
//! wait on `rooms::event_handler`.

mod append;
mod data;

use std::sync::Arc;

use futures::{Future, Stream, TryStreamExt, pin_mut};
pub use phantom_core::matrix::pdu::{PduId, RawPduId};
use phantom_core::{
    Result, at, err,
    future::TryExt,
    implement,
    matrix::pdu::{PduCount, PduEvent},
    stream::TryIgnore,
    sync::MutexMap,
};
use ruma::{CanonicalJsonObject, EventId, OwnedRoomId, RoomId, UserId};

use self::data::Data;
pub use self::data::PdusIterItem;
use crate::{Dep, account_data, admin, appservice, rooms, sending, server_state, users};

pub struct Service {
    /// Held across assigning an event its position in a room and writing it
    /// there.
    ///
    /// Distinct from the state mutex a caller already holds: that one orders
    /// changes to a room's *state*, and this one orders the counter. An event
    /// with no state key never touches the first and still must not share a
    /// position with another.
    mutex_insert: RoomMutexMap,
    services: Services,
    db: Data,
}

struct Services {
    account_data: Dep<account_data::Service>,
    admin: Dep<admin::Service>,
    alias: Dep<rooms::alias::Service>,
    appservice: Dep<appservice::Service>,
    pdu_metadata: Dep<rooms::pdu_metadata::Service>,
    pusher: Dep<crate::pusher::Service>,
    read_receipt: Dep<rooms::read_receipt::Service>,
    search: Dep<rooms::search::Service>,
    sending: Dep<sending::Service>,
    server_state: Dep<server_state::Service>,
    short: Dep<rooms::short::Service>,
    spaces: Dep<rooms::spaces::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    threads: Dep<rooms::threads::Service>,
    user: Dep<rooms::user::Service>,
    users: Dep<users::Service>,
}

type RoomMutexMap = MutexMap<OwnedRoomId, ()>;

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            mutex_insert: RoomMutexMap::new(),
            services: Services {
                account_data: args.depend::<account_data::Service>("account_data"),
                admin: args.depend::<admin::Service>("admin"),
                alias: args.depend::<rooms::alias::Service>("rooms::alias"),
                appservice: args.depend::<appservice::Service>("appservice"),
                pdu_metadata: args.depend::<rooms::pdu_metadata::Service>("rooms::pdu_metadata"),
                pusher: args.depend::<crate::pusher::Service>("pusher"),
                read_receipt: args.depend::<rooms::read_receipt::Service>("rooms::read_receipt"),
                search: args.depend::<rooms::search::Service>("rooms::search"),
                sending: args.depend::<sending::Service>("sending"),
                server_state: args.depend::<server_state::Service>("server_state"),
                short: args.depend::<rooms::short::Service>("rooms::short"),
                spaces: args.depend::<rooms::spaces::Service>("rooms::spaces"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                threads: args.depend::<rooms::threads::Service>("rooms::threads"),
                user: args.depend::<rooms::user::Service>("rooms::user"),
                users: args.depend::<users::Service>("users"),
            },
            db: Data::new(&args),
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn first_pdu_in_room(&self, room_id: &RoomId) -> Result<PduEvent> {
    self.first_item_in_room(room_id).await.map(at!(1))
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn first_item_in_room(&self, room_id: &RoomId) -> Result<(PduCount, PduEvent)> {
    let pdus = self.pdus(None, room_id, None);

    pin_mut!(pdus);
    pdus.try_next()
        .await?
        .ok_or_else(|| err!(Request(NotFound("No PDU found in room"))))
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn latest_pdu_in_room(&self, room_id: &RoomId) -> Result<PduEvent> {
    self.db.latest_pdu_in_room(None, room_id).await
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn last_timeline_count(
    &self,
    sender_user: Option<&UserId>,
    room_id: &RoomId,
) -> Result<PduCount> {
    self.db.last_timeline_count(sender_user, room_id).await
}

/// Returns the `count` of this pdu's id.
#[implement(Service)]
pub async fn get_pdu_count(&self, event_id: &EventId) -> Result<PduCount> {
    self.db.get_pdu_count(event_id).await
}

/// Returns the json of a pdu.
#[implement(Service)]
pub async fn get_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db.get_pdu_json(event_id).await
}

/// Returns the json of a pdu, without checking the outliers.
#[implement(Service)]
#[inline]
pub async fn get_non_outlier_pdu_json(&self, event_id: &EventId) -> Result<CanonicalJsonObject> {
    self.db.get_non_outlier_pdu_json(event_id).await
}

/// Returns the pdu's id.
#[implement(Service)]
#[inline]
pub async fn get_pdu_id(&self, event_id: &EventId) -> Result<RawPduId> {
    self.db.get_pdu_id(event_id).await
}

/// Returns the pdu, without checking the outliers.
#[implement(Service)]
#[inline]
pub async fn get_non_outlier_pdu(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db.get_non_outlier_pdu(event_id).await
}

/// Returns the pdu.
///
/// Checks `eventid_outlierpdu` if it is not found in the timeline.
#[implement(Service)]
pub async fn get_pdu(&self, event_id: &EventId) -> Result<PduEvent> {
    self.db.get_pdu(event_id).await
}

/// Returns the pdu.
///
/// This does __NOT__ check the outliers.
#[implement(Service)]
pub async fn get_pdu_from_id(&self, pdu_id: &RawPduId) -> Result<PduEvent> {
    self.db.get_pdu_from_id(pdu_id).await
}

/// Returns the pdu as a `BTreeMap<String, CanonicalJsonValue>`.
#[implement(Service)]
pub async fn get_pdu_json_from_id(&self, pdu_id: &RawPduId) -> Result<CanonicalJsonObject> {
    self.db.get_pdu_json_from_id(pdu_id).await
}

/// Whether the pdu exists.
///
/// Checks `eventid_outlierpdu` if it is not found in the timeline.
#[implement(Service)]
pub fn pdu_exists<'a>(&'a self, event_id: &'a EventId) -> impl Future<Output = bool> + Send + 'a {
    self.db.pdu_exists(event_id).is_ok()
}

/// Removes a pdu and creates a new one with the same id.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn replace_pdu(
    &self,
    pdu_id: &RawPduId,
    pdu_json: &CanonicalJsonObject,
    pdu: &PduEvent,
) -> Result<()> {
    self.db.replace_pdu(pdu_id, pdu_json, pdu).await
}

/// Returns an iterator over all PDUs in a room. Unknown rooms produce no
/// items.
#[implement(Service)]
#[inline]
pub fn all_pdus<'a>(
    &'a self,
    user_id: &'a UserId,
    room_id: &'a RoomId,
) -> impl Stream<Item = PdusIterItem> + Send + 'a {
    self.pdus(Some(user_id), room_id, None).ignore_err()
}

/// Reverse iteration starting at `until`.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn pdus_rev<'a>(
    &'a self,
    user_id: Option<&'a UserId>,
    room_id: &'a RoomId,
    until: Option<PduCount>,
) -> impl Stream<Item = Result<PdusIterItem>> + Send + 'a {
    self.db
        .pdus_rev(user_id, room_id, until.unwrap_or_else(PduCount::max))
}

/// Forward iteration starting at `from`.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub fn pdus<'a>(
    &'a self,
    user_id: Option<&'a UserId>,
    room_id: &'a RoomId,
    from: Option<PduCount>,
) -> impl Stream<Item = Result<PdusIterItem>> + Send + 'a {
    self.db
        .pdus(user_id, room_id, from.unwrap_or_else(PduCount::min))
}
