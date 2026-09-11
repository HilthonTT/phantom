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

const UPDATE_CHANNEL_CAP: usize = 100;

pub struct Service {
    server: Arc<Server>,
    services: Services,

    typing: RwLock<BTreeMap<OwnedRoomId, RoomTyping>>,

    pub typing_update_sender: broadcast::Sender<OwnedRoomId>,
}

struct Services {
    account_data: Dep<account_data::Service>,
    sending: Dep<sending::Service>,
    server_state: Dep<server_state::Service>,
}

#[derive(Default)]
struct RoomTyping {
    users: BTreeMap<OwnedUserId, u64>,

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

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn typing_add(&self, user_id: &UserId, room_id: &RoomId, timeout: u64) -> Result {
    debug!("typing started {user_id:?} in {room_id:?} timeout:{timeout:?}");

    {
        let mut typing = self.typing.write().await;
        let count = self.services.server_state.next_count()?;
        let room = typing.entry(room_id.to_owned()).or_default();
        room.users.insert(user_id.to_owned(), timeout);
        room.update = count;
    }

    self.announce(room_id);
    self.broadcast(room_id, user_id, true).await
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub async fn typing_remove(&self, user_id: &UserId, room_id: &RoomId) -> Result {
    debug!("typing stopped {user_id:?} in {room_id:?}");

    {
        let mut typing = self.typing.write().await;
        let count = self.services.server_state.next_count()?;
        let room = typing.entry(room_id.to_owned()).or_default();
        room.users.remove(user_id);
        room.update = count;
    }

    self.announce(room_id);
    self.broadcast(room_id, user_id, false).await
}

#[implement(Service)]
pub async fn wait_for_update(&self, room_id: &RoomId) {
    let mut receiver = self.typing_update_sender.subscribe();

    while let Ok(next) = receiver.recv().await {
        if next == room_id {
            break;
        }
    }
}

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

#[implement(Service)]
pub async fn typing_users_for_user(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
) -> Result<Vec<OwnedUserId>> {
    let user_ids = self.typing_users(room_id).await;

    Ok(self.filter_ignored(user_ids, sender_user).await)
}

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

#[implement(Service)]
async fn typings_maintain(&self, room_id: &RoomId) -> Result {
    let now = time::now_millis();

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

        if removed.is_empty() {
            return Ok(());
        }

        room.update = self.services.server_state.next_count()?;
    }

    for user_id in &removed {
        debug!("typing timeout {user_id:?} in {room_id:?}");
    }

    self.announce(room_id);

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

#[implement(Service)]
fn announce(&self, room_id: &RoomId) {
    if self.typing_update_sender.send(room_id.to_owned()).is_err() {
        trace!("nothing is waiting on typing updates for {room_id:?}");
    }
}

#[implement(Service)]
async fn broadcast(&self, room_id: &RoomId, user_id: &UserId, typing: bool) -> Result {
    let appservices = self.appservice_send(room_id);

    let federation = async {
        if self.services.server_state.user_is_local(user_id) {
            self.federation_send(room_id, user_id, typing).await
        } else {
            Ok(())
        }
    };

    try_join(appservices, federation).await.map(|((), ())| ())
}

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

#[implement(Service)]
async fn federation_send(&self, room_id: &RoomId, user_id: &UserId, typing: bool) -> Result {
    debug_assert!(
        self.services.server_state.user_is_local(user_id),
        "tried to broadcast the typing status of a remote user",
    );

    if !self.server.config.presence.allow_outgoing_typing {
        return Ok(());
    }

    let content = TypingContent::new(room_id.to_owned(), user_id.to_owned(), typing);
    let edu = Edu::Typing(content);

    let mut buf = EduBuf::new();
    serde_json::to_writer(&mut buf, &edu).expect("failed to serialize Typing EDU to JSON");

    self.services.sending.send_edu_room(room_id, buf).await
}
