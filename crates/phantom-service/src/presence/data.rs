//! Where a user's presence is kept.
//!
//! Two columns: `userid_presenceid` points a user at the number their current
//! presence was written under, and `presenceid_presence` holds the record
//! itself under that number. The number comes from the server counter, so
//! `presenceid_presence` is in write order and a sync can read everything
//! that changed after the token it last saw without walking every user.
//!
//! A write therefore inserts under a fresh number and deletes the record the
//! old number pointed at, rather than overwriting in place.

use std::sync::Arc;

use futures::Stream;
use phantom_core::{
    Result,
    bytes::u64_from_u8,
    debug_warn, err,
    stream::{ReadyExt, TryIgnore},
    text::str_from_bytes,
    time,
};
use phantom_database::{Deserialized, Json, Map};
use ruma::{UInt, UserId, events::presence::PresenceEvent, presence::PresenceState};

use super::Presence;
use crate::{Dep, server_state, users};

pub(crate) struct Data {
    presenceid_presence: Arc<Map>,
    userid_presenceid: Arc<Map>,
    services: Services,
}

struct Services {
    server_state: Dep<server_state::Service>,
    users: Dep<users::Service>,
}

impl Data {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let db = &args.db;
        Self {
            presenceid_presence: db["presenceid_presence"].clone(),
            userid_presenceid: db["userid_presenceid"].clone(),
            services: Services {
                server_state: args.depend::<server_state::Service>("server_state"),
                users: args.depend::<users::Service>("users"),
            },
        }
    }

    pub(super) async fn get_presence(&self, user_id: &UserId) -> Result<(u64, PresenceEvent)> {
        let count = self
            .userid_presenceid
            .get(user_id)
            .await
            .deserialized::<u64>()?;

        let key = presenceid_key(count, user_id);
        let bytes = self.presenceid_presence.get(&key).await?;
        let event = Presence::from_json_bytes(&bytes)?
            .to_presence_event(user_id, &self.services.users)
            .await;

        Ok((count, event))
    }

    pub(super) async fn set_presence(
        &self,
        user_id: &UserId,
        presence_state: &PresenceState,
        currently_active: Option<bool>,
        last_active_ago: Option<UInt>,
        status_msg: Option<String>,
    ) -> Result<()> {
        let last_presence = self.get_presence(user_id).await;
        let state_changed = match last_presence {
            Err(_) => true,
            Ok((_, ref presence)) => presence.content.presence != *presence_state,
        };

        let status_msg_changed = match last_presence {
            Err(_) => true,
            Ok((_, ref last_presence)) => {
                let old_msg = last_presence.content.status_msg.clone().unwrap_or_default();
                let new_msg = status_msg.clone().unwrap_or_default();

                new_msg != old_msg
            }
        };

        let now = time::now_millis();
        let last_last_active_ts = match last_presence {
            Err(_) => 0,
            Ok((_, ref presence)) => {
                now.saturating_sub(presence.content.last_active_ago.unwrap_or_default().into())
            }
        };

        let last_active_ts = match last_active_ago {
            None => now,
            Some(last_active_ago) => now.saturating_sub(last_active_ago.into()),
        };

        if !status_msg_changed && !state_changed && last_active_ts < last_last_active_ts {
            debug_warn!(
                "presence spam {user_id:?} last_active_ts:{last_active_ts:?} < \
				 {last_last_active_ts:?}",
            );
            return Ok(());
        }

        let status_msg = if status_msg.as_ref().is_some_and(String::is_empty) {
            None
        } else {
            status_msg
        };

        let presence = Presence::new(
            presence_state.to_owned(),
            currently_active.unwrap_or(false),
            last_active_ts,
            status_msg,
        );

        let count = self.services.server_state.next_count()?;
        let key = presenceid_key(count, user_id);

        self.presenceid_presence.raw_put(key, Json(presence))?;
        self.userid_presenceid.raw_put(user_id, count)?;

        if let Ok((last_count, _)) = last_presence {
            let key = presenceid_key(last_count, user_id);
            self.presenceid_presence.remove(&key)?;
        }

        Ok(())
    }

    pub(super) async fn remove_presence(&self, user_id: &UserId) -> Result<()> {
        let Ok(count) = self
            .userid_presenceid
            .get(user_id)
            .await
            .deserialized::<u64>()
        else {
            return Ok(());
        };

        let key = presenceid_key(count, user_id);
        self.presenceid_presence.remove(&key)?;
        self.userid_presenceid.remove(user_id)?;

        Ok(())
    }

    #[inline]
    pub(super) fn presence_since(
        &self,
        since: u64,
    ) -> impl Stream<Item = (&UserId, u64, &[u8])> + Send + '_ {
        self.presenceid_presence
            .raw_stream()
            .ignore_err()
            .ready_filter_map(move |(key, presence)| {
                let (count, user_id) = presenceid_parse(key).ok()?;
                (count > since).then_some((user_id, count, presence))
            })
    }
}

#[inline]
fn presenceid_key(count: u64, user_id: &UserId) -> Vec<u8> {
    let cap = size_of::<u64>().saturating_add(user_id.as_bytes().len());
    let mut key = Vec::with_capacity(cap);
    key.extend_from_slice(&count.to_be_bytes());
    key.extend_from_slice(user_id.as_bytes());
    key
}

#[inline]
fn presenceid_parse(key: &[u8]) -> Result<(u64, &UserId)> {
    let (count, user_id) = key
        .split_at_checked(size_of::<u64>())
        .ok_or_else(|| err!(Database("Presence key is too short to hold a count")))?;

    let user_id = user_id_from_bytes(user_id)?;
    let count = u64_from_u8(count);

    Ok((count, user_id))
}

/// Parses a `UserId` from bytes.
fn user_id_from_bytes(bytes: &[u8]) -> Result<&UserId> {
    let str: &str = str_from_bytes(bytes)?;
    let user_id: &UserId = str.try_into()?;

    Ok(user_id)
}
