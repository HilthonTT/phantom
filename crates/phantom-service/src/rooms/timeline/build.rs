//! Building an event of this server's own.
//!
//! This is the half of the write path that [`append`](super::append) is the
//! other end of. An event that arrives over federation is already an event —
//! it has an id, a sender's signature and a place in the room's graph. One of
//! this server's own is none of those things until it is built here: given
//! the room's current forward extremities as its parents, given the auth
//! events its type and sender demand, checked against those, hashed, signed,
//! and only then handed to `append_pdu` to become part of the room.
//!
//! The order is the spec's and is not ours to rearrange. The auth check runs
//! against the event as built but *before* it is signed, because a rejected
//! event must never carry this server's signature. The event id is derived
//! from the signed canonical JSON, which is why it cannot be filled in until
//! the end and is a placeholder until then.
//!
//! [`build_and_append_pdu`](super::Service::build_and_append_pdu) is the
//! entry point. It holds the room's state mutex across the whole of building,
//! appending and installing the new state, which is what makes it the only
//! way a room's state changes at this server's own initiative.

use std::{borrow::Borrow, cmp, collections::HashSet, iter::once};

use futures::{
    FutureExt, StreamExt, TryStreamExt,
    future::{self, ready},
};
use phantom_core::{
    Err, Error, Result, err, implement,
    json::to_canonical_object,
    matrix::{
        Event,
        pdu::{EventHash, PduBuilder, PduEvent, gen_event_id},
        state_res::{self, RoomVersion},
    },
    stream::{IterStream, ReadyExt, TryIgnore},
    time::now_millis,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedServerName, RoomId, RoomVersionId,
    UserId,
    canonical_json::to_canonical_value,
    events::{
        StateEventType, TimelineEventType,
        room::{
            create::RoomCreateEventContent,
            member::{MembershipState, RoomMemberEventContent},
            redaction::RoomRedactionEventContent,
        },
    },
    uint,
};
use serde_json::value::to_raw_value;

use super::Service;
use crate::rooms::state::RoomMutexGuard;

/// How many of the room's forward extremities a new event names as parents.
///
/// A room that has forked badly can have a great many, and naming all of them
/// would make the event enormous for no gain — the point of naming several is
/// to heal the fork, and twenty is as much healing as one event needs to do.
const MAX_PREV_EVENTS: usize = 20;

/// Builds a PDU from `pdu_builder`, authorizes it, and signs it.
///
/// Returns the event and the canonical JSON it was signed as. The two are
/// kept side by side from here on: the JSON is what other servers are given
/// and what the signature covers, and the struct is what this server reads.
/// Regenerating one from the other would not be safe, since a round trip
/// through the struct need not reproduce the exact bytes that were signed.
///
/// The mutex guard is taken and not used. It is the caller's proof that the
/// room's state cannot move between the auth check here and the append that
/// follows it.
#[implement(Service)]
pub async fn create_hash_and_sign_event(
    &self,
    pdu_builder: PduBuilder,
    sender: &UserId,
    room_id: &RoomId,
    _mutex_lock: &RoomMutexGuard,
) -> Result<(PduEvent, CanonicalJsonObject)> {
    let PduBuilder {
        event_type,
        content,
        unsigned,
        state_key,
        redacts,
        timestamp,
    } = pdu_builder;

    let prev_events: Vec<OwnedEventId> = self
        .services
        .state
        .get_forward_extremities(room_id)
        .take(MAX_PREV_EVENTS)
        .map(Into::into)
        .collect()
        .await;

    // A room's version is read from its create event, which the create event
    // itself cannot do — it is what establishes the version, so it carries it
    // in its own content.
    let room_version_id = match self.services.state.get_room_version(room_id).await {
        Ok(room_version_id) => room_version_id,
        Err(_) if event_type == TimelineEventType::RoomCreate => {
            let content: RoomCreateEventContent = serde_json::from_str(content.get())?;
            content.room_version
        }
        Err(_) => {
            return Err(Error::InconsistentRoomState(
                "non-create event for room of unknown version",
                room_id.to_owned(),
            ));
        }
    };

    let room_version = RoomVersion::new(&room_version_id).expect("room version is supported");

    let auth_events = self
        .services
        .state
        .get_auth_events(room_id, &event_type, sender, state_key.as_deref(), &content)
        .await?;

    let depth = prev_events
        .iter()
        .stream()
        .map(Ok)
        .and_then(|event_id| self.get_pdu(event_id))
        .and_then(|pdu| future::ok(pdu.depth))
        .ignore_err()
        .ready_fold(uint!(0), cmp::max)
        .await
        .saturating_add(uint!(1));

    let mut unsigned = unsigned.unwrap_or_default();

    // A state event carries what it replaced, so that a client which sees only
    // the new event can still tell what changed.
    if let Some(state_key) = &state_key
        && let Ok(prev_pdu) = self
            .services
            .state_accessor
            .room_state_get(room_id, &event_type.to_string().into(), state_key)
            .await
    {
        unsigned.insert("prev_content".to_owned(), prev_pdu.get_content_as_value());
        unsigned.insert(
            "prev_sender".to_owned(),
            serde_json::to_value(&prev_pdu.sender).expect("UserId::to_value always works"),
        );
        unsigned.insert(
            "replaces_state".to_owned(),
            serde_json::to_value(&prev_pdu.event_id).expect("EventId is valid json"),
        );
    }

    let mut pdu = PduEvent {
        event_id: ruma::event_id!("$thiswillbefilledinlater").into(),
        room_id: room_id.to_owned(),
        sender: sender.to_owned(),
        origin: None,
        origin_server_ts: timestamp.map_or_else(
            || now_millis().try_into().expect("u64 fits into UInt"),
            |ts| ts.get(),
        ),
        kind: event_type,
        content,
        state_key,
        prev_events,
        depth,
        auth_events: auth_events
            .values()
            .map(|pdu| pdu.event_id.clone())
            .collect(),
        redacts,
        unsigned: if unsigned.is_empty() {
            None
        } else {
            Some(to_raw_value(&unsigned).expect("to_raw_value always works"))
        },
        hashes: EventHash {
            sha256: "aaa".to_owned(),
        },
        signatures: None,
    };

    let auth_fetch = |k: &StateEventType, s: &str| {
        let key = (k.clone(), s.into());
        ready(auth_events.get(&key))
    };

    let auth_check = state_res::auth_check(&room_version, &pdu, None, auth_fetch)
        .await
        .map_err(|e| err!(Request(Forbidden(warn!("Auth check failed: {e:?}")))))?;

    if !auth_check {
        return Err!(Request(Forbidden("Event is not authorized.")));
    }

    let mut pdu_json = to_canonical_object(&pdu).map_err(|e| {
        err!(Request(BadJson(warn!(
            "Failed to convert PDU to canonical JSON: {e}"
        ))))
    })?;

    // Only the oldest room versions carry the event id in the event itself.
    // Everywhere else it is derived from the rest, so leaving it in would put
    // the placeholder above under the signature.
    match room_version_id {
        RoomVersionId::V1 | RoomVersionId::V2 => {}
        _ => {
            pdu_json.remove("event_id");
        }
    }

    pdu_json.insert(
        "origin".to_owned(),
        to_canonical_value(self.services.server_state.server_name())
            .expect("server name is a valid CanonicalJsonValue"),
    );

    if let Err(e) = self
        .services
        .server_keys
        .hash_and_sign_event(&mut pdu_json, &room_version_id)
    {
        return match e {
            Error::SignaturesJson(ruma::signatures::JsonError::PduTooLarge) => {
                Err!(Request(TooLarge(
                    "Message/PDU is too long (exceeds 65535 bytes)"
                )))
            }
            _ => Err!(Request(Unknown(warn!("Signing event failed: {e}")))),
        };
    }

    pdu.event_id = gen_event_id(&pdu_json, &room_version_id)?;

    pdu_json.insert(
        "event_id".into(),
        CanonicalJsonValue::String(pdu.event_id.clone().into()),
    );

    // Claim the short id now so that everything downstream of the append can
    // assume the event has one.
    let _shorteventid = self
        .services
        .short
        .get_or_create_shorteventid(&pdu.event_id)
        .await;

    Ok((pdu, pdu_json))
}

/// Creates a new persisted data unit and adds it to a room.
///
/// Takes the room's state mutex, which is what makes this the only way the
/// room's state changes at this server's initiative: building the event,
/// writing it, and installing the state it produced are one step, and a
/// second caller cannot interleave with them.
#[implement(Service)]
#[tracing::instrument(skip(self, state_lock), level = "debug")]
pub async fn build_and_append_pdu(
    &self,
    pdu_builder: PduBuilder,
    sender: &UserId,
    room_id: &RoomId,
    state_lock: &RoomMutexGuard,
) -> Result<OwnedEventId> {
    let (pdu, pdu_json) = self
        .create_hash_and_sign_event(pdu_builder, sender, room_id, state_lock)
        .await?;

    if self.services.admin.is_admin_room(&pdu.room_id).await {
        self.check_pdu_for_admin_room(&pdu, sender).boxed().await?;
    }

    // Redaction is authorized separately from the auth rules: whether a user
    // may redact a *particular* event depends on who sent that event, which
    // the auth rules do not look at. Where the redacted id lives moved in room
    // version 11, from the event to its content.
    if pdu.kind == TimelineEventType::RoomRedaction {
        use RoomVersionId::*;

        let redacts = match self.services.state.get_room_version(&pdu.room_id).await? {
            V1 | V2 | V3 | V4 | V5 | V6 | V7 | V8 | V9 | V10 => pdu.redacts.clone(),
            _ => {
                let content: RoomRedactionEventContent = pdu.get_content()?;
                content.redacts
            }
        };

        if let Some(redact_id) = &redacts
            && !self
                .services
                .state_accessor
                .user_can_redact(redact_id, &pdu.sender, &pdu.room_id, false)
                .await?
        {
            return Err!(Request(Forbidden("User cannot redact this event.")));
        }
    }

    if pdu.kind == TimelineEventType::RoomMember {
        let content: RoomMemberEventContent = pdu.get_content()?;

        if content.join_authorized_via_users_server.is_some()
            && content.membership != MembershipState::Join
        {
            return Err!(Request(BadJson(
                "join_authorised_via_users_server is only for member joins"
            )));
        }

        if content
            .join_authorized_via_users_server
            .as_ref()
            .is_some_and(|authorising_user| {
                !self.services.server_state.user_is_local(authorising_user)
            })
        {
            return Err!(Request(InvalidParam(
                "Authorising user does not belong to this homeserver"
            )));
        }
    }

    let statehashid = self.services.state.append_to_state(&pdu).await?;

    let pdu_id = self
        .append_pdu(&pdu, pdu_json, once(pdu.event_id.borrow()), state_lock)
        .boxed()
        .await?;

    self.services
        .state
        .set_room_state(&pdu.room_id, statehashid, state_lock);

    let mut servers: HashSet<OwnedServerName> = self
        .services
        .state_cache
        .room_servers(&pdu.room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    // A membership event has to reach the server of whoever it is about, even
    // when that server is not in the room — a leave or a ban is exactly the
    // case where they are about to stop being in it.
    if pdu.kind == TimelineEventType::RoomMember
        && let Some(state_key_uid) = &pdu
            .state_key
            .as_ref()
            .and_then(|state_key| UserId::parse(state_key.as_str()).ok())
    {
        servers.insert(state_key_uid.server_name().to_owned());
    }

    servers.remove(self.services.server_state.server_name());

    self.services
        .sending
        .send_pdu_servers(servers.iter().map(AsRef::as_ref).stream(), &pdu_id)
        .await?;

    Ok(pdu.event_id)
}

/// The extra rules that keep the admin room usable.
///
/// The admin room is how an operator reaches the server, so it may not be
/// encrypted — the server user has no keys to read it with — and it may not be
/// emptied of local admins, which would lock everyone out of it for good. The
/// server user itself can never leave or be banned.
#[implement(Service)]
#[tracing::instrument(skip_all, level = "debug")]
async fn check_pdu_for_admin_room(&self, pdu: &PduEvent, sender: &UserId) -> Result<()> {
    match &pdu.kind {
        TimelineEventType::RoomEncryption => {
            return Err!(Request(Forbidden(error!(
                "Encryption not supported in admins room."
            ))));
        }
        TimelineEventType::RoomMember => {
            let target = pdu
                .state_key()
                .filter(|v| v.starts_with('@'))
                .unwrap_or(sender.as_str());

            let server_user = &self.services.server_state.server_user.to_string();

            let content: RoomMemberEventContent = pdu.get_content()?;
            match content.membership {
                MembershipState::Leave => {
                    if target == server_user {
                        return Err!(Request(Forbidden(error!(
                            "Server user cannot leave the admins room."
                        ))));
                    }

                    if self.remaining_admins(pdu, target).await < 2 {
                        return Err!(Request(Forbidden(error!(
                            "Last admin cannot leave the admins room."
                        ))));
                    }
                }

                MembershipState::Ban if pdu.state_key().is_some() => {
                    if target == server_user {
                        return Err!(Request(Forbidden(error!(
                            "Server cannot be banned from admins room."
                        ))));
                    }

                    if self.remaining_admins(pdu, target).await < 2 {
                        return Err!(Request(Forbidden(error!(
                            "Last admin cannot be banned from admins room."
                        ))));
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }

    Ok(())
}

/// How many local members the admin room would have left without `target`.
#[implement(Service)]
async fn remaining_admins(&self, pdu: &PduEvent, target: &str) -> usize {
    self.services
        .state_cache
        .room_members(&pdu.room_id)
        .ready_filter(|user| self.services.server_state.user_is_local(user))
        .ready_filter(|user| *user != target)
        .boxed()
        .count()
        .await
}
