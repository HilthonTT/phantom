use std::sync::Arc;

use arrayvec::ArrayVec;
use futures::{Stream, StreamExt};
use phantom_core::{
    Result,
    arrayvec::ArrayVecExt,
    implement,
    matrix::pdu::{PduCount, PduEvent},
    set,
    stream::{IterStream, ReadyExt, TryIgnore, WidebandExt},
};
use phantom_database::{Map, SEP, keyval::Val};
use ruma::{RoomId, UserId, api::client::search::search_events::v3::Criteria};

use crate::{
    Dep, rooms,
    rooms::{
        short::ShortRoomId,
        timeline::{PduId, RawPduId},
    },
};

pub struct Service {
    db: Data,
    services: Services,
}

struct Data {
    tokenids: Arc<Map>,
}

struct Services {
    short: Dep<rooms::short::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    timeline: Dep<rooms::timeline::Service>,
}

#[derive(Clone, Debug)]
pub struct RoomQuery<'a> {
    pub room_id: &'a RoomId,
    pub user_id: Option<&'a UserId>,
    pub criteria: &'a Criteria,
    pub limit: usize,
    pub skip: usize,
}

type TokenId = ArrayVec<u8, TOKEN_ID_MAX_LEN>;

const TOKEN_ID_MAX_LEN: usize = size_of::<ShortRoomId>() + WORD_MAX_LEN + 1 + size_of::<RawPduId>();

const WORD_MAX_LEN: usize = 50;

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Data {
                tokenids: args.db["tokenids"].clone(),
            },
            services: Services {
                short: args.depend::<rooms::short::Service>("rooms::short"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

#[implement(Service)]
pub fn index_pdu(&self, shortroomid: ShortRoomId, pdu_id: &RawPduId, message_body: &str) -> Result {
    self.db.tokenids.insert_batch(
        tokenize(message_body)
            .map(|word| make_tokenid(shortroomid, &word, pdu_id))
            .map(|key| (key, [])),
    )
}

#[implement(Service)]
pub fn deindex_pdu(
    &self,
    shortroomid: ShortRoomId,
    pdu_id: &RawPduId,
    message_body: &str,
) -> Result {
    for word in tokenize(message_body) {
        self.db
            .tokenids
            .remove(&make_tokenid(shortroomid, &word, pdu_id))?;
    }

    Ok(())
}

#[implement(Service)]
#[tracing::instrument(skip(self), level = "debug")]
pub(super) async fn delete_all_tokenids(&self, shortroomid: ShortRoomId) {
    self.db.tokenids.del_prefix(&shortroomid).await;
}

#[implement(Service)]
pub async fn search_pdus<'a>(
    &'a self,
    query: &'a RoomQuery<'a>,
) -> Result<(usize, impl Stream<Item = PduEvent> + Send + 'a)> {
    let pdu_ids: Vec<_> = self.search_pdu_ids(query).await?.collect().await;

    let count = pdu_ids.len();
    let pdus = pdu_ids
        .into_iter()
        .stream()
        .wide_filter_map(move |result_pdu_id: RawPduId| async move {
            self.services
                .timeline
                .get_pdu_from_id(&result_pdu_id)
                .await
                .ok()
        })
        .ready_filter(|pdu| !pdu.is_redacted())
        .ready_filter(|pdu| pdu.matches(&query.criteria.filter))
        .wide_filter_map(move |pdu| async move {
            self.services
                .state_accessor
                .user_can_see_event(query.user_id?, &pdu.room_id, &pdu.event_id)
                .await
                .then_some(pdu)
        })
        .skip(query.skip)
        .take(query.limit);

    Ok((count, pdus))
}

#[implement(Service)]
pub async fn search_pdu_ids(
    &self,
    query: &RoomQuery<'_>,
) -> Result<impl Stream<Item = RawPduId> + Send + '_ + use<'_>> {
    let shortroomid = self.services.short.get_shortroomid(query.room_id).await?;

    let pdu_ids = self.search_pdu_ids_query_room(query, shortroomid).await;

    let iters = pdu_ids.into_iter().map(IntoIterator::into_iter);

    Ok(set::intersection(iters).stream())
}

#[implement(Service)]
async fn search_pdu_ids_query_room(
    &self,
    query: &RoomQuery<'_>,
    shortroomid: ShortRoomId,
) -> Vec<Vec<RawPduId>> {
    tokenize(&query.criteria.search_term)
        .stream()
        .wide_then(|word| async move {
            self.search_pdu_ids_query_words(shortroomid, &word)
                .collect::<Vec<_>>()
                .await
        })
        .collect::<Vec<_>>()
        .await
}

#[implement(Service)]
fn search_pdu_ids_query_words<'a>(
    &'a self,
    shortroomid: ShortRoomId,
    word: &'a str,
) -> impl Stream<Item = RawPduId> + Send + 'a {
    self.search_pdu_ids_query_word(shortroomid, word)
        .map(move |key| -> RawPduId { key[prefix_len(word)..].into() })
}

#[implement(Service)]
fn search_pdu_ids_query_word(
    &self,
    shortroomid: ShortRoomId,
    word: &str,
) -> impl Stream<Item = Val<'_>> + Send + '_ + use<'_> {
    let end_id: RawPduId = PduId {
        shortroomid,
        shorteventid: PduCount::max(),
    }
    .into();

    let end = make_tokenid(shortroomid, word, &end_id);
    let prefix = make_prefix(shortroomid, word);

    self.db
        .tokenids
        .rev_raw_keys_from(&end)
        .ignore_err()
        .ready_take_while(move |key| key.starts_with(&prefix))
}

fn tokenize(body: &str) -> impl Iterator<Item = String> + Send + '_ {
    body.split_terminator(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .filter(|word| word.len() <= WORD_MAX_LEN)
}

fn make_tokenid(shortroomid: ShortRoomId, word: &str, pdu_id: &RawPduId) -> TokenId {
    let mut key = make_prefix(shortroomid, word);
    key.extend_from_slice(pdu_id.as_ref());
    key
}

fn make_prefix(shortroomid: ShortRoomId, word: &str) -> TokenId {
    let mut key = TokenId::new();
    key.extend_from_slice(&shortroomid.to_be_bytes());
    key.extend_from_slice(word.as_bytes());
    key.push(SEP);
    key
}

fn prefix_len(word: &str) -> usize {
    size_of::<ShortRoomId>()
        .saturating_add(word.len())
        .saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::{WORD_MAX_LEN, make_prefix, tokenize};

    #[test]
    fn tokens_are_limited_after_lowercasing() {
        let word = "İ".repeat(WORD_MAX_LEN / 2);
        assert!(word.len() <= WORD_MAX_LEN);

        for token in tokenize(&word) {
            assert!(token.len() <= WORD_MAX_LEN);
            make_prefix(0, &token);
        }
    }
}
