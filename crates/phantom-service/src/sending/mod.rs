//! Outbound federation, appservice, and push traffic.
//!
//! Everything the server sends elsewhere is queued here and pushed out by a
//! pool of worker tasks. A destination always lands on the same worker, which
//! is what keeps the transactions to one server in order. Queued events are
//! persisted, so what is in flight when the server stops goes out at the
//! next start.

mod appservice;
mod data;
mod dest;
mod sender;

use std::{
    fmt::Debug,
    hash::{DefaultHasher, Hash, Hasher},
    iter::once,
    sync::Arc,
};

use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use futures::{Stream, StreamExt, stream::FuturesUnordered};
use phantom_core::{
    Result, err, math::usize_from_u64_truncated, result::LogErr, server::Server, stream::ReadyExt,
};
use ruma::{RoomId, ServerName, UserId};
use smallvec::SmallVec;

use self::data::Data;
pub use self::dest::Destination;
use crate::{
    Dep, account_data, appservice as appservice_service, client, federation, presence, pusher,
    rooms, rooms::timeline::RawPduId, server_state, users,
};

pub struct Service {
    pub db: Data,
    server: Arc<Server>,
    services: Services,
    channels: Vec<(Sender<Msg>, Receiver<Msg>)>,
}

struct Services {
    client: Dep<client::Service>,
    server_state: Dep<server_state::Service>,
    state: Dep<rooms::state::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    user: Dep<rooms::user::Service>,
    users: Dep<users::Service>,
    presence: Dep<presence::Service>,
    read_receipt: Dep<rooms::read_receipt::Service>,
    timeline: Dep<rooms::timeline::Service>,
    account_data: Dep<account_data::Service>,
    appservice: Dep<appservice_service::Service>,
    pusher: Dep<pusher::Service>,
    federation: Dep<federation::Service>,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SendingEvent {
    Pdu(RawPduId), // pduid
    Edu(EduBuf),   // edu json
    Flush,         // none
}

pub type EduBuf = SmallVec<[u8; EDU_BUF_CAP]>;
pub type EduVec = SmallVec<[EduBuf; EDU_VEC_CAP]>;

const EDU_BUF_CAP: usize = 128;
const EDU_VEC_CAP: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Msg {
    dest: Destination,
    event: SendingEvent,
    queue_id: Vec<u8>,
}

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let num_senders = num_senders(&args);

        Ok(Arc::new(Self {
            db: Data::new(&args),
            server: args.server.clone(),
            services: Services {
                client: args.depend::<client::Service>("client"),
                server_state: args.depend::<server_state::Service>("server_state"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                user: args.depend::<rooms::user::Service>("rooms::user"),
                users: args.depend::<users::Service>("users"),
                presence: args.depend::<presence::Service>("presence"),
                read_receipt: args.depend::<rooms::read_receipt::Service>("rooms::read_receipt"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
                account_data: args.depend::<account_data::Service>("account_data"),
                appservice: args.depend::<appservice_service::Service>("appservice"),
                pusher: args.depend::<pusher::Service>("pusher"),
                federation: args.depend::<federation::Service>("federation"),
            },
            channels: (0..num_senders)
                .map(|_| async_channel::unbounded())
                .collect(),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        let mut senders = self
            .channels
            .iter()
            .enumerate()
            .map(|(id, _)| self.clone().sender(id))
            .collect::<FuturesUnordered<_>>();

        while let Some(result) = senders.next().await {
            result?;
        }

        Ok(())
    }

    fn interrupt(&self) {
        for (sender, _) in &self.channels {
            sender.close();
        }
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    #[tracing::instrument(skip(self, pdu_id, user, pushkey), level = "debug")]
    pub fn send_pdu_push(&self, pdu_id: &RawPduId, user: &UserId, pushkey: String) -> Result {
        let dest = Destination::Push(user.to_owned(), pushkey);
        let event = SendingEvent::Pdu(*pdu_id);
        let _cork = self.db.db.engine.cork_guard();
        let keys = self.db.queue_requests(once((&event, &dest)))?;
        self.dispatch(Msg {
            dest,
            event,
            queue_id: keys.into_iter().next().expect("request queue results"),
        })
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub fn send_pdu_appservice(&self, appservice_id: String, pdu_id: RawPduId) -> Result {
        let dest = Destination::Appservice(appservice_id);
        let event = SendingEvent::Pdu(pdu_id);
        let _cork = self.db.db.engine.cork_guard();
        let keys = self.db.queue_requests(once((&event, &dest)))?;
        self.dispatch(Msg {
            dest,
            event,
            queue_id: keys.into_iter().next().expect("request queue results"),
        })
    }

    #[tracing::instrument(skip(self, room_id, pdu_id), level = "debug")]
    pub async fn send_pdu_room(&self, room_id: &RoomId, pdu_id: &RawPduId) -> Result {
        let servers = self
            .services
            .state_cache
            .room_servers(room_id)
            .ready_filter(|server_name| !self.services.server_state.server_is_ours(server_name));

        self.send_pdu_servers(servers, pdu_id).await
    }

    #[tracing::instrument(skip(self, servers, pdu_id), level = "debug")]
    pub async fn send_pdu_servers<'a, S>(&self, servers: S, pdu_id: &RawPduId) -> Result
    where
        S: Stream<Item = &'a ServerName> + Send + 'a,
    {
        let requests = servers
            .map(|server| {
                (
                    Destination::Federation(server.into()),
                    SendingEvent::Pdu(*pdu_id),
                )
            })
            .collect::<Vec<_>>()
            .await;

        let _cork = self.db.db.engine.cork_guard();
        let keys = self
            .db
            .queue_requests(requests.iter().map(|(o, e)| (e, o)))?;

        for ((dest, event), queue_id) in requests.into_iter().zip(keys) {
            self.dispatch(Msg {
                dest,
                event,
                queue_id,
            })?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, room_id, serialized), level = "debug")]
    pub async fn send_edu_room(&self, room_id: &RoomId, serialized: EduBuf) -> Result {
        let servers = self
            .services
            .state_cache
            .room_servers(room_id)
            .ready_filter(|server_name| !self.services.server_state.server_is_ours(server_name));

        self.send_edu_servers(servers, serialized).await
    }

    #[tracing::instrument(skip(self, servers, serialized), level = "debug")]
    pub async fn send_edu_servers<'a, S>(&self, servers: S, serialized: EduBuf) -> Result
    where
        S: Stream<Item = &'a ServerName> + Send + 'a,
    {
        let requests = servers
            .map(|server| {
                (
                    Destination::Federation(server.to_owned()),
                    SendingEvent::Edu(serialized.clone()),
                )
            })
            .collect::<Vec<_>>()
            .await;

        let _cork = self.db.db.engine.cork_guard();
        let keys = self
            .db
            .queue_requests(requests.iter().map(|(o, e)| (e, o)))?;

        for ((dest, event), queue_id) in requests.into_iter().zip(keys) {
            self.dispatch(Msg {
                dest,
                event,
                queue_id,
            })?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self, serialized), level = "debug")]
    pub fn send_edu_push(&self, user: &UserId, pushkey: String, serialized: EduBuf) -> Result {
        let dest = Destination::Push(user.to_owned(), pushkey);
        let event = SendingEvent::Edu(serialized);
        let _cork = self.db.db.engine.cork_guard();
        let keys = self.db.queue_requests(once((&event, &dest)))?;
        self.dispatch(Msg {
            dest,
            event,
            queue_id: keys.into_iter().next().expect("request queue results"),
        })
    }

    #[tracing::instrument(skip(self, serialized), level = "debug")]
    pub fn send_edu_appservice(&self, appservice_id: String, serialized: EduBuf) -> Result {
        let dest = Destination::Appservice(appservice_id);
        let event = SendingEvent::Edu(serialized);
        let _cork = self.db.db.engine.cork_guard();
        let keys = self.db.queue_requests(once((&event, &dest)))?;
        self.dispatch(Msg {
            dest,
            event,
            queue_id: keys.into_iter().next().expect("request queue results"),
        })
    }

    #[tracing::instrument(skip(self, room_id), level = "debug")]
    pub async fn flush_room(&self, room_id: &RoomId) -> Result {
        let servers = self
            .services
            .state_cache
            .room_servers(room_id)
            .ready_filter(|server_name| !self.services.server_state.server_is_ours(server_name));

        self.flush_servers(servers).await
    }

    #[tracing::instrument(skip(self, servers), level = "debug")]
    pub async fn flush_servers<'a, S>(&self, servers: S) -> Result
    where
        S: Stream<Item = &'a ServerName> + Send + 'a,
    {
        servers
            .map(ToOwned::to_owned)
            .map(Destination::Federation)
            .map(|dest| Msg {
                dest,
                event: SendingEvent::Flush,
                queue_id: Vec::<u8>::new(),
            })
            .ready_for_each(|msg| {
                self.dispatch(msg).log_err().ok();
            })
            .await;

        Ok(())
    }

    /// Cleans up the queued and in-flight requests for every destination
    /// that starts with `prefix`. What it runs over is the raw queue key, so
    /// the only sensible callers are the ones removing a server or an
    /// appservice.
    pub async fn cleanup_events(
        &self,
        appservice_id: Option<String>,
        user_id: Option<&UserId>,
        push_key: Option<&str>,
    ) -> Result {
        match (appservice_id, user_id, push_key) {
            (None, Some(user_id), Some(push_key)) => {
                self.db
                    .delete_all_requests_for(&Destination::Push(
                        user_id.to_owned(),
                        push_key.to_owned(),
                    ))
                    .await;

                Ok(())
            }
            (Some(appservice_id), None, None) => {
                self.db
                    .delete_all_requests_for(&Destination::Appservice(appservice_id))
                    .await;

                Ok(())
            }
            _ => {
                debug_assert!(
                    false,
                    "cleanup_events called with too many or too few arguments"
                );
                Ok(())
            }
        }
    }

    fn dispatch(&self, msg: Msg) -> Result {
        let shard = self.shard_id(&msg.dest);
        let sender = &self
            .channels
            .get(shard)
            .expect("missing sender worker channels")
            .0;

        debug_assert!(!sender.is_full(), "channel full");
        debug_assert!(!sender.is_closed(), "channel closed");
        sender.try_send(msg).map_err(|e| err!("{e}"))
    }

    /// Which worker this destination's traffic goes through.
    pub(super) fn shard_id(&self, dest: &Destination) -> usize {
        shard_id(dest, self.channels.len())
    }
}

/// [`Service::shard_id`] against a given number of workers.
///
/// The destination is hashed rather than read as an integer out of the leading
/// bytes of its queue prefix. Those bytes are not a `u64` to begin with — the
/// prefix of the appservice `irc` is the five bytes `+irc\xFF` — and they are
/// not a spread either, since every appservice prefix opens with the same
/// sigil and the servers of one hosting provider share a suffix, not a prefix.
///
/// What matters is only that a destination always comes out the same, which is
/// what keeps one server's transactions on one worker and so in order.
fn shard_id(dest: &Destination, senders: usize) -> usize {
    if senders <= 1 {
        return 0;
    }

    let mut hasher = DefaultHasher::new();
    dest.hash(&mut hasher);

    usize_from_u64_truncated(hasher.finish()) % senders
}

fn num_senders(args: &crate::Args<'_>) -> usize {
    const MIN_SENDERS: usize = 1;

    // Limit the number of senders to the number of workers threads or number of
    // cores, conservatively.
    let max_senders = args
        .server
        .metrics
        .num_workers()
        .min(phantom_core::sys::compute::available_parallelism())
        .max(MIN_SENDERS);

    // If the user doesn't override the default 0, this is intended to then
    // default to 1 for now as multiple senders is experimental.
    args.server
        .config
        .sender_workers
        .clamp(MIN_SENDERS, max_senders)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use ruma::{OwnedServerName, OwnedUserId};

    use super::{Destination, shard_id};

    fn federation(server: &str) -> Destination {
        Destination::Federation(OwnedServerName::try_from(server).expect("valid server name"))
    }

    fn push(user: &str, pushkey: &str) -> Destination {
        Destination::Push(
            OwnedUserId::try_from(user).expect("valid user id"),
            pushkey.to_owned(),
        )
    }

    /// A queue prefix is not eight bytes long just because a `u64` is: the
    /// appservice `irc` has the five-byte prefix `+irc\xFF`, and so does the
    /// server `a.io`. Reading a shard out of the first eight bytes of one of
    /// those is reading past the end of it.
    #[test]
    fn short_destinations_shard_like_any_other() {
        let dests = [
            Destination::Appservice("irc".to_owned()),
            federation("a.io"),
            push("@a:b.io", "k"),
        ];

        for dest in dests {
            for senders in 1..=8 {
                assert!(
                    shard_id(&dest, senders) < senders,
                    "{dest:?} over {senders} senders"
                );
            }
        }
    }

    /// One server's transactions stay ordered only because they all go through
    /// the same worker, so the shard has to be a function of the destination
    /// alone.
    #[test]
    fn a_destination_always_lands_on_the_same_worker() {
        let dest = federation("matrix.org");
        let first = shard_id(&dest, 4);

        for _ in 0..8 {
            assert_eq!(shard_id(&federation("matrix.org"), 4), first);
        }
    }

    /// Every appservice prefix opens with the same sigil and every push prefix
    /// with another, so a shard taken from the leading bytes would pile each
    /// kind onto one worker.
    #[test]
    fn destinations_of_one_kind_spread_over_the_workers() {
        const SENDERS: usize = 4;

        let appservices: HashSet<usize> = ["irc", "telegram", "discord", "slack", "xmpp", "sms"]
            .into_iter()
            .map(|id| shard_id(&Destination::Appservice(id.to_owned()), SENDERS))
            .collect();

        assert!(appservices.len() > 1, "every appservice on one worker");

        let pushers: HashSet<usize> = ["@a:b.io", "@c:d.io", "@e:f.io", "@g:h.io", "@i:j.io"]
            .into_iter()
            .map(|user| shard_id(&push(user, "key"), SENDERS))
            .collect();

        assert!(pushers.len() > 1, "every pusher on one worker");
    }

    /// A single worker takes everything, and is the default.
    #[test]
    fn one_worker_takes_every_destination() {
        assert_eq!(shard_id(&federation("matrix.org"), 1), 0);
        assert_eq!(shard_id(&Destination::Appservice("irc".to_owned()), 1), 0);
        assert_eq!(shard_id(&push("@a:b.io", "k"), 0), 0);
    }
}
