//! Who is typing in each room.
//!
//! Typing is the one piece of room state that is never written down. A
//! notification is worth nothing a moment after it was sent, and a restart
//! that forgets every indicator is indistinguishable from one where everybody
//! stopped typing — so this is a map in memory and no column at all.
//!
//! Each indicator carries the timestamp it expires at, supplied by the client.
//! Nothing runs on a timer to collect them: expiry is applied by whatever next
//! reads the room, in [`typings_maintain`], because an indicator nobody is
//! looking at costs nothing by still being there.
//!
//! Every change draws a number from the server counter and records it as the
//! room's `update`. That is what a sync compares against its token to know
//! whether a room's typing list has moved, and `typing_update_sender` is what
//! wakes a sync that is waiting rather than polling.
//!
//! [`typings_maintain`]: Service::typings_maintain

use std::{collections::BTreeMap, sync::Arc};

use futures::future::try_join;
use phantom_core::{Result, debug, implement, server::Server, time, trace};
use ruma::{
    OwnedRoomId, OwnedUserId, RoomId, UserId,
    api::{
        appservice::event::push_events::v1::EphemeralData,
        federation::transactions::edu::{Edu, TypingContent},
    },
    events::{
        EphemeralRoomEvent, GlobalAccountDataEventType, ignored_user_list::IgnoredUserListEvent,
        typing::TypingEventContent,
    },
};
use tokio::sync::{RwLock, broadcast};

use crate::{Dep, account_data, sending, sending::EduBuf, server_state};

/// How many room updates may be in flight to a waiting sync before the slowest
/// one starts missing them. A missed wake-up costs a sync the wait it was in,
/// not the update itself — the count is read from the map afterwards either
/// way.
const UPDATE_CHANNEL_CAP: usize = 100;

pub struct Service {
    server: Arc<Server>,
    services: Services,

    /// Who is typing where. One lock over every room rather than one per room:
    /// a hold is a map lookup and a clone of a short list of user ids, and a
    /// second level of locking would cost more than it saves.
    typing: RwLock<BTreeMap<OwnedRoomId, RoomTyping>>,

    /// Announces the rooms whose typing list has changed, for
    /// [`wait_for_update`](Service::wait_for_update).
    pub typing_update_sender: broadcast::Sender<OwnedRoomId>,
}

struct Services {
    account_data: Dep<account_data::Service>,
    sending: Dep<sending::Service>,
    server_state: Dep<server_state::Service>,
}

/// One room's typing indicators.
#[derive(Default)]
struct RoomTyping {
    /// Each user, and the Unix-epoch millisecond their indicator expires at.
    users: BTreeMap<OwnedUserId, u64>,

    /// The counter value of the last change to this room.
    update: u64,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            server: args.server.clone(),
            services: Services {
                account_data: args.depend::<account_data::Service>("account_data"),
                sending: args.depend::<sending::Service>("sending"),
                server_state: args.depend::<server_state::Service>("server_state"),
            },
            typing: RwLock::new(BTreeMap::new()),
            typing_update_sender: broadcast::channel(UPDATE_CHANNEL_CAP).0,
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

/// Marks a user as typing in a room until `timeout`, or until
/// [`typing_remove`] takes it back.
///
/// [`typing_remove`]: Service::typing_remove
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn typing_add(&self, user_id: &UserId, room_id: &RoomId, timeout: u64) -> Result {
    debug!("typing started {user_id:?} in {room_id:?} timeout:{timeout:?}");

    {
        // The count is drawn under the same lock the change is made under, so
        // that two writers racing cannot stamp the later change with the
        // earlier number and have a sync skip it.
        let mut typing = self.typing.write().await;
        let count = self.services.server_state.next_count()?;
        let room = typing.entry(room_id.to_owned()).or_default();
        room.users.insert(user_id.to_owned(), timeout);
        room.update = count;
    }

    self.announce(room_id);
    self.broadcast(room_id, user_id, true).await
}

/// Takes a user's typing indicator back before its timeout.
#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn typing_remove(&self, user_id: &UserId, room_id: &RoomId) -> Result {
    debug!("typing stopped {user_id:?} in {room_id:?}");

    {
        // See `typing_add` on why the count is drawn here.
        let mut typing = self.typing.write().await;
        let count = self.services.server_state.next_count()?;
        let room = typing.entry(room_id.to_owned()).or_default();
        room.users.remove(user_id);
        room.update = count;
    }

    self.announce(room_id);
    self.broadcast(room_id, user_id, false).await
}

/// Waits until the room's typing list changes.
#[implement(Service)]
pub async fn wait_for_update(&self, room_id: &RoomId) {
    let mut receiver = self.typing_update_sender.subscribe();

    while let Ok(next) = receiver.recv().await {
        if next == room_id {
            break;
        }
    }
}

/// The counter value of the last change to the room's typing list, expired
/// indicators having first been collected.
#[implement(Service)]
pub async fn last_typing_update(&self, room_id: &RoomId) -> Result<u64> {
    self.typings_maintain(room_id).await?;

    Ok(self
        .typing
        .read()
        .await
        .get(room_id)
        .map_or(0, |room| room.update))
}

/// Who is typing in the room, as `sender_user` should see it.
#[implement(Service)]
pub async fn typing_users_for_user(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
) -> Result<Vec<OwnedUserId>> {
    let user_ids = self.typing_users(room_id).await;

    Ok(self.filter_ignored(user_ids, sender_user).await)
}

/// One update token and the users visible at it, read together.
///
/// A sync needs both halves to agree: reading the token and then the users
/// under separate locks can return a token from before a change with the users
/// from after it, which makes the next sync skip the update. `select` is given
/// the token under the same lock and returns false to say the caller has
/// already seen it, so a sync that is not going to send anything does not pay
/// for the user list.
#[implement(Service)]
pub async fn typing_snapshot_for_user<Select>(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    select: Select,
) -> Result<Option<(u64, Vec<OwnedUserId>)>>
where
    Select: FnOnce(u64) -> bool + Send,
{
    self.typings_maintain(room_id).await?;

    let (update, user_ids) = {
        let typing = self.typing.read().await;
        let room = typing.get(room_id);
        let update = room.map_or(0, |room| room.update);

        if !select(update) {
            return Ok(None);
        }

        let user_ids: Vec<_> = room
            .into_iter()
            .flat_map(|room| room.users.keys().cloned())
            .collect();

        (update, user_ids)
    };

    Ok(Some((
        update,
        self.filter_ignored(user_ids, sender_user).await,
    )))
}

/// Drops the indicators whose timeout has passed, and tells everyone that
/// cares if any were.
///
/// Called on the read paths rather than from a timer: an expired indicator
/// only matters when somebody asks who is typing, and collecting it then means
/// no task per room.
#[implement(Service)]
async fn typings_maintain(&self, room_id: &RoomId) -> Result {
    let now = time::now_millis();

    // Checked under the read lock first, because the overwhelmingly common
    // answer is that nothing has expired and no writer needs to be let in.
    let expired = self
        .typing
        .read()
        .await
        .get(room_id)
        .is_some_and(|room| room.users.values().any(|timeout| *timeout < now));

    if !expired {
        return Ok(());
    }

    let mut removed = Vec::new();

    {
        let mut typing = self.typing.write().await;
        let Some(room) = typing.get_mut(room_id) else {
            return Ok(());
        };

        room.users.retain(|user_id, timeout| {
            let live = *timeout >= now;
            if !live {
                removed.push(user_id.clone());
            }

            live
        });

        // Another reader may have collected the same expiries between the two
        // locks, in which case this one has nothing to announce and has not
        // spent a count saying so.
        if removed.is_empty() {
            return Ok(());
        }

        room.update = self.services.server_state.next_count()?;
    }

    for user_id in &removed {
        debug!("typing timeout {user_id:?} in {room_id:?}");
    }

    self.announce(room_id);

    // One EDU per user rather than one for the room: the federation form of a
    // typing notification names a single user, so a batch of expiries is a
    // batch of EDUs.
    let federation = async {
        for user_id in &removed {
            if self.services.server_state.user_is_local(user_id) {
                self.federation_send(room_id, user_id, false).await?;
            }
        }

        Ok(())
    };

    try_join(self.appservice_send(room_id), federation)
        .await
        .map(|((), ())| ())
}

/// Everyone typing in the room, unfiltered.
#[implement(Service)]
async fn typing_users(&self, room_id: &RoomId) -> Vec<OwnedUserId> {
    self.typing
        .read()
        .await
        .get(room_id)
        .into_iter()
        .flat_map(|room| room.users.keys().cloned())
        .collect()
}

/// Drops the users `sender_user` has ignored.
///
/// Read after the typing lock is released: it is a database read, and holding
/// the map against every reader for the length of one is what would make this
/// service contended.
#[implement(Service)]
async fn filter_ignored(
    &self,
    user_ids: Vec<OwnedUserId>,
    sender_user: &UserId,
) -> Vec<OwnedUserId> {
    if user_ids.is_empty() {
        return user_ids;
    }

    let ignored: Option<IgnoredUserListEvent> = self
        .services
        .account_data
        .get_global(sender_user, GlobalAccountDataEventType::IgnoredUserList)
        .await
        .ok();

    let Some(ignored) = ignored else {
        return user_ids;
    };

    user_ids
        .into_iter()
        .filter(|user_id| {
            !ignored
                .content
                .ignored_users
                .contains_key(user_id.as_ref() as &UserId)
        })
        .collect()
}

/// Wakes anything waiting on this room in [`wait_for_update`].
///
/// No receivers is the ordinary case — nothing is syncing this room — so the
/// error it reports is not one.
///
/// [`wait_for_update`]: Service::wait_for_update
#[implement(Service)]
fn announce(&self, room_id: &RoomId) {
    if self.typing_update_sender.send(room_id.to_owned()).is_err() {
        trace!("nothing is waiting on typing updates for {room_id:?}");
    }
}

/// Tells the appservices and the other servers about one user's change.
#[implement(Service)]
async fn broadcast(&self, room_id: &RoomId, user_id: &UserId, typing: bool) -> Result {
    let appservices = self.appservice_send(room_id);

    let federation = async {
        if self.services.server_state.user_is_local(user_id) {
            self.federation_send(room_id, user_id, typing).await
        } else {
            // A remote user's indicator reached us over federation already;
            // sending it back out would be this server speaking for a user
            // that is not its own.
            Ok(())
        }
    };

    try_join(appservices, federation).await.map(|((), ())| ())
}

/// Sends the room's whole typing list to the appservices watching it.
///
/// Appservices are told who is typing rather than what changed, which is the
/// client-facing shape of the event and the only one the appservice API has.
#[implement(Service)]
async fn appservice_send(&self, room_id: &RoomId) -> Result {
    let user_ids = self.typing_users(room_id).await;
    let content = TypingEventContent::new(user_ids);

    self.services
        .sending
        .send_edu_room_appservices(room_id, |buf| {
            let edu =
                EphemeralData::Typing(EphemeralRoomEvent::new(room_id.to_owned(), content.clone()));

            serde_json::to_writer(buf, &edu)?;

            Ok(())
        })
        .await
}

/// Sends one user's change to the other servers in the room.
#[implement(Service)]
async fn federation_send(&self, room_id: &RoomId, user_id: &UserId, typing: bool) -> Result {
    debug_assert!(
        self.services.server_state.user_is_local(user_id),
        "tried to broadcast the typing status of a remote user",
    );

    if !self.server.config.allow_outgoing_typing {
        return Ok(());
    }

    let content = TypingContent::new(room_id.to_owned(), user_id.to_owned(), typing);
    let edu = Edu::Typing(content);

    let mut buf = EduBuf::new();
    serde_json::to_writer(&mut buf, &edu).expect("failed to serialize Typing EDU to JSON");

    self.services.sending.send_edu_room(room_id, buf).await
}
