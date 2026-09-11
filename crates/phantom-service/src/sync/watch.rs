use std::collections::HashSet;

use futures::{FutureExt, StreamExt, future::select_all, pin_mut, stream::FuturesUnordered};
use phantom_core::{Result, implement, trace};
use phantom_database::{Interfix, serialize_to_vec};
use ruma::{DeviceId, UserId};

use super::Service;

#[implement(Service)]
#[tracing::instrument(name = "watch", level = "debug", skip(self))]
pub async fn watch(&self, user_id: &UserId, device_id: &DeviceId) -> Result {
    let mut waiters = Vec::new();

    let user_prefix = serialize_to_vec((user_id, Interfix))?;
    let device_prefix = serialize_to_vec((user_id, device_id, Interfix))?;

    waiters.push(
        self.db["todeviceid_events"]
            .watch_prefix(&device_prefix)
            .boxed(),
    );

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

        waiters.push(
            self.db["keychangeid_userid"]
                .watch_prefix(&room_prefix)
                .boxed(),
        );

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
