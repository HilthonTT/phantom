//! Parking a sync request until there is something to answer it with.
//!
//! A client asks for everything that has happened since a position it names.
//! When nothing has, the request is not answered with an empty response — it
//! is held open until something does happen, and answered then. That is what
//! makes a message appear in a client the moment it is sent rather than at the
//! client's next poll.
//!
//! Holding it open means knowing what "something happens" covers, and the
//! honest answer is: every column a sync response is drawn from. So this
//! registers a waiter on each of them, under the narrowest prefix that could
//! concern this user, and returns as soon as any one fires. Waking on
//! something the response turns out not to include is harmless — the request
//! is answered, finds nothing, and is made again — while *not* waking on
//! something it would have included is a message that never arrives.
//!
//! Two things here are not database columns. Typing notifications live in
//! memory and are waited on through their own broadcast, and the server's
//! shutdown is waited on so that a client parked here does not hold the
//! process open.

use std::collections::HashSet;

use futures::{FutureExt, StreamExt, future::select_all, pin_mut, stream::FuturesUnordered};
use phantom_core::{Result, implement, trace};
use phantom_database::{Interfix, serialize_to_vec};
use ruma::{DeviceId, UserId};

use super::Service;

/// Returns once anything a sync for this device would report has changed.
///
/// Also returns on shutdown, so that a parked client does not keep the server
/// from stopping. A caller cannot tell the two apart and does not need to: the
/// answer either way is to make the response it would have made.
#[implement(Service)]
#[tracing::instrument(name = "watch", level = "debug", skip(self))]
pub async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result {
    let mut waiters = Vec::new();

    let user_prefix = serialize_to_vec((user_id, Interfix))?;
    let device_prefix = serialize_to_vec((user_id, device_id, Interfix))?;

    // Anything sent to this device in particular.
    waiters.push(
        self.db["todeviceid_events"]
            .watch_prefix(&device_prefix)
            .boxed(),
    );

    // The rooms the user is in, and how: a room they have just been invited to
    // or removed from changes the response whether or not anything happened
    // inside it.
    for column in [
        "userroomid_joined",
        "userroomid_invitestate",
        "userroomid_leftstate",
        "userroomid_knockedstate",
        "userroomid_notificationcount",
        "userroomid_highlightcount",
    ] {
        waiters.push(self.db[column].watch_prefix(&user_prefix).boxed());
    }

    // Their own device list, which changes when they log in elsewhere, and the
    // one-time key counts a client tracks alongside it.
    waiters.push(
        self.db["userid_devicelistversion"]
            .watch_prefix(user_id.as_bytes())
            .boxed(),
    );
    waiters.push(
        self.db["userid_lastonetimekeyupdate"]
            .watch_prefix(user_id.as_bytes())
            .boxed(),
    );

    // Their presence, which they see reflected back.
    waiters.push(
        self.db["userid_presenceid"]
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

    // Account data is keyed by room first, and the global kind is stored under
    // no room at all, so it takes one waiter per room plus one for the rest.
    waiters.push(
        self.db["roomusertype_roomuserdataid"]
            .watch_prefix(&serialize_to_vec((Interfix, user_id, Interfix))?)
            .boxed(),
    );

    let mut typing = FuturesUnordered::new();

    for room_id in &rooms {
        let room_user_prefix = serialize_to_vec((room_id, user_id, Interfix))?;
        let room_prefix = serialize_to_vec((room_id, Interfix))?;

        waiters.push(
            self.db["roomusertype_roomuserdataid"]
                .watch_prefix(&room_user_prefix)
                .boxed(),
        );

        waiters.push(
            self.db["roomuserid_lastprivatereadupdate"]
                .watch_prefix(&room_user_prefix)
                .boxed(),
        );

        waiters.push(
            self.db["readreceiptid_readreceipt"]
                .watch_prefix(&room_prefix)
                .boxed(),
        );

        // Someone in the room changing their keys is a device list update for
        // everyone who shares it with them.
        waiters.push(
            self.db["keychangeid_userid"]
                .watch_prefix(&room_prefix)
                .boxed(),
        );

        // New events in the room. Keyed by the room's short id, which is what
        // the timeline is stored under.
        if let Ok(shortroomid) = self.services.short.get_shortroomid(room_id).await {
            waiters.push(
                self.db["pduid_pdu"]
                    .watch_prefix(&serialize_to_vec(shortroomid)?)
                    .boxed(),
            );
        }

        typing.push(self.services.typing.wait_for_update(room_id));
    }

    trace!(
        waiters = waiters.len(),
        rooms = rooms.len(),
        "Parked on sync"
    );

    let typing = typing.next();
    let shutdown = self.services.server.until_shutdown();

    pin_mut!(typing, shutdown);

    tokio::select! {
        () = select_all(waiters).map(|_| ()) => {},
        _ = typing => {},
        () = shutdown => {},
    }

    Ok(())
}

/// Every user whose device list this user would be told about: themselves, and
/// everyone they share a room with.
///
/// Kept beside [`watch`] because it answers the same question from the other
/// side — a caller that has been woken by `keychangeid_userid` needs to know
/// which of the changes were its business.
///
/// [`watch`]: Service::watch
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

    // A deactivated account's keys are not going to change again, and a client
    // that asks after them is told nothing rather than told about a user who
    // is gone.
    let mut active = HashSet::with_capacity(interest.len());

    for candidate in interest {
        if self.services.users.exists(&candidate).await {
            active.insert(candidate);
        }
    }

    active
}
