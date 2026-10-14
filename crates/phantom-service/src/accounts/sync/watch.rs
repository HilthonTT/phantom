use std::{collections::HashSet, future::Future, pin::Pin};

use futures::{FutureExt, StreamExt, future::select_all};
use phantom_core::{Result, implement, trace};
use phantom_database::{Interfix, serialize_to_vec, table};
use ruma::{DeviceId, OwnedRoomId, RoomId, UserId};
use tokio::sync::broadcast::error::RecvError;

use super::Service;

/// A sync's armed wait: it resolves on the first write that may change what
/// the user would be sent, on a typing update in one of their rooms, or on
/// shutdown.
pub type Watch = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Arms every wake source a sync of this user and device waits on.
///
/// Every watcher is registered by the time this returns, so a sync must call
/// it *before* it samples the global count it answers up to: a write landing
/// after that sample, while the response is being built, then wakes the
/// returned future instead of leaving the long-poll parked until its timeout.
/// The future is `'static` so it can be held across the response build.
#[implement(Service)]
#[tracing::instrument(name = "watch", level = "debug", skip(self))]
pub async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result<Watch> {
    let mut waiters: Vec<Watch> = Vec::new();

    // Subscribed before anything is read, for the same reason as the
    // database watchers below.
    let mut typing_updates = self.services.typing.typing_update_sender.subscribe();

    let user_prefix = serialize_to_vec((user_id, Interfix))?;
    let device_prefix = serialize_to_vec((user_id, device_id, Interfix))?;

    waiters.push(
        self.db[table::TODEVICEID_EVENTS]
            .watch_prefix(&device_prefix)
            .boxed(),
    );

    for column in [
        table::USERROOMID_JOINED,
        table::USERROOMID_INVITESTATE,
        table::USERROOMID_LEFTSTATE,
        table::USERROOMID_KNOCKEDSTATE,
        table::USERROOMID_NOTIFICATIONCOUNT,
        table::USERROOMID_HIGHLIGHTCOUNT,
    ] {
        waiters.push(self.db[column].watch_prefix(&user_prefix).boxed());
    }

    waiters.push(
        self.db[table::USERID_DEVICELISTVERSION]
            .watch_prefix(user_id.as_bytes())
            .boxed(),
    );
    waiters.push(
        self.db[table::USERID_LASTONETIMEKEYUPDATE]
            .watch_prefix(user_id.as_bytes())
            .boxed(),
    );

    waiters.push(
        self.db[table::KEYCHANGEID_USERID]
            .watch_prefix(&user_prefix)
            .boxed(),
    );

    waiters.push(
        self.db[table::USERID_PRESENCEID]
            .watch_prefix(user_id.as_bytes())
            .boxed(),
    );

    let rooms: Vec<_> = self
        .services
        .state_cache
        .rooms_joined(user_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    waiters.push(
        self.db[table::ROOMUSERTYPE_ROOMUSERDATAID]
            .watch_prefix(&serialize_to_vec((None::<&RoomId>, user_id, Interfix))?)
            .boxed(),
    );

    for room_id in &rooms {
        let room_user_prefix = serialize_to_vec((room_id, user_id, Interfix))?;
        let room_prefix = serialize_to_vec((room_id, Interfix))?;

        waiters.push(
            self.db[table::ROOMUSERTYPE_ROOMUSERDATAID]
                .watch_prefix(&room_user_prefix)
                .boxed(),
        );

        waiters.push(
            self.db[table::ROOMUSERID_LASTPRIVATEREADUPDATE]
                .watch_prefix(&room_user_prefix)
                .boxed(),
        );

        waiters.push(
            self.db[table::READRECEIPTID_READRECEIPT]
                .watch_prefix(&room_prefix)
                .boxed(),
        );

        waiters.push(
            self.db[table::KEYCHANGEID_USERID]
                .watch_prefix(&room_prefix)
                .boxed(),
        );

        if let Ok(shortroomid) = self.services.short.get_shortroomid(room_id).await {
            waiters.push(
                self.db[table::PDUID_PDU]
                    .watch_prefix(&serialize_to_vec(shortroomid)?)
                    .boxed(),
            );
        }
    }

    trace!(
        waiters = waiters.len(),
        rooms = rooms.len(),
        "Armed sync watchers"
    );

    let rooms: HashSet<OwnedRoomId> = rooms.into_iter().collect();
    waiters.push(
        async move {
            loop {
                match typing_updates.recv().await {
                    Ok(room_id) if !rooms.contains(&room_id) => {}
                    // A lagged receiver may have missed one of the user's rooms.
                    Ok(_) | Err(RecvError::Lagged(_)) => break,
                    Err(RecvError::Closed) => futures::future::pending().await,
                }
            }
        }
        .boxed(),
    );

    let server = self.services.server.clone();
    waiters.push(async move { server.until_shutdown().await }.boxed());

    Ok(async move {
        select_all(waiters).await;
    }
    .boxed())
}

#[implement(Service)]
pub async fn device_list_interest(&self, user_id: &UserId) -> HashSet<ruma::OwnedUserId> {
    let mut interest: HashSet<_> = [user_id.to_owned()].into_iter().collect();

    let rooms: Vec<_> = self
        .services
        .state_cache
        .rooms_joined(user_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for room_id in rooms {
        let members: Vec<_> = self
            .services
            .state_cache
            .room_members(&room_id)
            .map(ToOwned::to_owned)
            .collect()
            .await;

        interest.extend(members);
    }

    let mut active = HashSet::with_capacity(interest.len());

    for candidate in interest {
        if self.services.users.exists(&candidate).await {
            active.insert(candidate);
        }
    }

    active
}
