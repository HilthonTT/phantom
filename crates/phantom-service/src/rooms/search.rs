//! Full-text search over the messages in a room.
//!
//! One inverted index, in the `tokenids` column: a message is split into words
//! and each word gets a key of `(shortroomid, word, pdu_id)` with an empty
//! value, so the words a query names can be looked up by prefix and the sets
//! of events they appear in intersected.
//!
//! The index is deliberately dumb — no stemming, no ranking, no phrase
//! matching. What a client asks for is which events in one room contain every
//! word it typed, newest first, which a prefix scan answers directly; anything
//! cleverer would have to be reindexed when it changed its mind.
//!
//! Only the events a user may see come back. The intersection is done on event
//! ids first and the visibility check second, because the check reads room
//! state per event and the intersection is what makes that a short list.

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

/// One room's worth of a client's search request.
///
/// A search spans rooms, but the index is per room and so is the visibility
/// check, so the caller splits its request into one of these per room.
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

/// Longer words are not indexed. A word that long is a URL, a hash or a
/// base64 blob rather than something anyone will search for, and the key is a
/// fixed-size buffer.
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

/// Adds a message to the index.
///
/// One batch rather than a write per word: a message is a bounded number of
/// tokens, and the whole message appearing in the index at once is what keeps
/// a concurrent search from matching half of it.
#[implement(Service)]
pub fn index_pdu(&self, shortroomid: ShortRoomId, pdu_id: &RawPduId, message_body: &str) -> Result {
    self.db.tokenids.insert_batch(
        tokenize(message_body)
            .map(|word| make_tokenid(shortroomid, &word, pdu_id))
            .map(|key| (key, [])),
    )
}

/// Removes a message from the index, which is what a redaction does to it.
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

/// The events in one room matching a query, and how many there were.
///
/// The count is of what the index matched, before the filter and the
/// visibility check, which is what upstream reports and what a client pages
/// through.
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

/// The ids of the events containing every word of the query.
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

/// One list of event ids per word of the query.
///
/// Collected rather than streamed, because intersecting them means holding all
/// of them at once anyway, and the shortest is what bounds the answer.
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

/// The events containing one word, newest first.
#[implement(Service)]
fn search_pdu_ids_query_words<'a>(
    &'a self,
    shortroomid: ShortRoomId,
    word: &'a str,
) -> impl Stream<Item = RawPduId> + Send + 'a {
    self.search_pdu_ids_query_word(shortroomid, word)
        .map(move |key| -> RawPduId { key[prefix_len(word)..].into() })
}

/// The raw keys of one word's entries, newest first.
///
/// Scanned backwards from the highest event id that key could have, since the
/// event id is the last part of the key and orders by when the event arrived.
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

/// Splits a string into the tokens used as keys in the inverted index.
///
/// The same function tokenizes a message being indexed and a query being run,
/// which is the only thing that makes the two agree on what a word is.
fn tokenize(body: &str) -> impl Iterator<Item = String> + Send + '_ {
    body.split_terminator(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .filter(|word| word.len() <= WORD_MAX_LEN)
        .map(str::to_lowercase)
}

/// The key one word of one event is indexed under.
///
/// The event id carries the short room id again, after the copy this key
/// starts with. That is redundant, and it is on disk: changing it would mean
/// reindexing every room.
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
