//! Threaded replies, and who has taken part in them.
//!
//! A thread is not a record of its own: it is a root event, and the replies
//! that point back at it with an `m.thread` relation. What is kept for it is
//! the summary a client wants before it opens one — how many replies there
//! are and what the latest is — folded into the root event's
//! `unsigned.m.relations` as each reply arrives, so a room's threads can be
//! listed without reading every reply in each of them.
//!
//! The one column here is the participant set, keyed by the root event's pdu
//! id: the users who have posted in that thread, root sender included. It is
//! written as the ids joined by the separator byte, which is how a sequence
//! is spelled in this database, so it reads back as a `Vec<OwnedUserId>`.
//!
//! `threads_until` walks a room's thread roots newest first. The spec's
//! `/threads` also honours `include`, returning only the threads the user has
//! participated in when asked for those; that filtering is not here yet, so
//! the parameter is ignored and every root in the room is returned.

use std::{collections::BTreeMap, sync::Arc};

use futures::{Stream, StreamExt};
use phantom_core::{
    Result, err,
    matrix::pdu::{PduCount, PduEvent, PduId, RawPduId},
    stream::{ReadyExt, TryIgnore, WidebandExt},
};
use phantom_database::{Deserialized, Map};
use ruma::{
    CanonicalJsonValue, EventId, OwnedUserId, RoomId, UserId,
    api::client::threads::get_threads::v1::IncludeThreads, events::relation::BundledThread, uint,
};

use crate::{Dep, rooms, rooms::short::ShortRoomId};

pub struct Service {
    db: Data,
    services: Services,
}

struct Services {
    short: Dep<rooms::short::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

pub(super) struct Data {
    threadid_userids: Arc<Map>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            db: Data {
                threadid_userids: args.db["threadid_userids"].clone(),
            },
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    pub async fn add_to_thread(&self, root_event_id: &EventId, pdu: &PduEvent) -> Result<()> {
        let root_id = self
            .services
            .timeline
            .get_pdu_id(root_event_id)
            .await
            .map_err(|e| {
                err!(Request(InvalidParam(
                    "Invalid event_id in thread message: {e:?}"
                )))
            })?;

        let root_pdu = self
            .services
            .timeline
            .get_pdu_from_id(&root_id)
            .await
            .map_err(|e| err!(Request(InvalidParam("Thread root not found: {e:?}"))))?;

        let mut root_pdu_json = self
            .services
            .timeline
            .get_pdu_json_from_id(&root_id)
            .await
            .map_err(|e| err!(Request(InvalidParam("Thread root pdu not found: {e:?}"))))?;

        if let CanonicalJsonValue::Object(unsigned) = root_pdu_json
            .entry("unsigned".to_owned())
            .or_insert_with(|| CanonicalJsonValue::Object(BTreeMap::default()))
        {
            let thread = unsigned
                .get("m.relations")
                .and_then(|relations| relations.as_object())
                .and_then(|relations| relations.get("m.thread"))
                .and_then(|thread| {
                    serde_json::from_value::<BundledThread>(thread.clone().into()).ok()
                })
                .map_or_else(
                    || BundledThread::new(pdu.to_sync_message_like_event(), uint!(1), true),
                    |mut thread| {
                        thread.count = thread.count.saturating_add(uint!(1));
                        thread.latest_event = pdu.to_sync_message_like_event();
                        thread
                    },
                );

            let thread: CanonicalJsonValue = serde_json::to_value(thread)
                .expect("to_value always works")
                .try_into()
                .expect("thread is valid json");

            match unsigned
                .entry("m.relations".to_owned())
                .or_insert_with(|| CanonicalJsonValue::Object(BTreeMap::default()))
            {
                CanonicalJsonValue::Object(relations) => {
                    relations.insert("m.thread".to_owned(), thread);
                }
                relations => {
                    *relations = CanonicalJsonValue::Object(BTreeMap::from([(
                        "m.thread".to_owned(),
                        thread,
                    )]));
                }
            }

            self.services
                .timeline
                .replace_pdu(&root_id, &root_pdu_json, &root_pdu)
                .await?;
        }

        let mut users = match self.get_participants(&root_id).await {
            Ok(userids) => userids,
            _ => vec![root_pdu.sender],
        };

        if !users.contains(&pdu.sender) {
            users.push(pdu.sender.clone());
        }

        self.update_participants(&root_id, &users)
    }

    pub async fn threads_until<'a>(
        &'a self,
        user_id: &'a UserId,
        room_id: &'a RoomId,
        shorteventid: PduCount,
        _inc: &'a IncludeThreads,
    ) -> Result<impl Stream<Item = (PduCount, PduEvent)> + Send + 'a> {
        let shortroomid: ShortRoomId = self.services.short.get_shortroomid(room_id).await?;

        let current: RawPduId = PduId {
            shortroomid,
            shorteventid: shorteventid.saturating_sub(1),
        }
        .into();

        let stream = self
            .db
            .threadid_userids
            .rev_raw_keys_from(&current)
            .ignore_err()
            .map(RawPduId::from)
            .ready_take_while(move |pdu_id| pdu_id.shortroomid() == shortroomid.to_be_bytes())
            .wide_filter_map(move |pdu_id| async move {
                let mut pdu = self.services.timeline.get_pdu_from_id(&pdu_id).await.ok()?;
                let pdu_id: PduId = pdu_id.into();

                if pdu.sender != user_id {
                    pdu.remove_transaction_id().ok();
                }

                Some((pdu_id.shorteventid, pdu))
            });

        Ok(stream)
    }

    pub(super) fn update_participants(
        &self,
        root_id: &RawPduId,
        participants: &[OwnedUserId],
    ) -> Result {
        let users = participants
            .iter()
            .map(|user| user.as_bytes())
            .collect::<Vec<_>>()
            .join(&[0xFF][..]);

        self.db.threadid_userids.insert(root_id, &users)
    }

    pub(super) async fn get_participants(&self, root_id: &RawPduId) -> Result<Vec<OwnedUserId>> {
        self.db.threadid_userids.get(root_id).await.deserialized()
    }
}
