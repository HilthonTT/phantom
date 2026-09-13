use std::{
    borrow::Borrow,
    collections::{BTreeSet, HashMap, HashSet},
    iter::once,
    mem::take,
    sync::Arc,
};

use futures::{FutureExt, StreamExt, stream};
use phantom_core::{
    Err, Result, debug, debug_error, debug_info, debug_warn, err, error, implement, info,
    matrix::{
        PduBuilder, PduEvent,
        pdu::gen_event_id_canonical_json,
        state_res::{self, RoomVersion},
    },
    rand::shuffle,
    trace, warn,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedServerName, OwnedUserId, RoomId,
    RoomOrAliasId, RoomVersionId, UserId,
    api::{
        error::ErrorKind,
        federation::{
            event::get_room_state,
            membership::{create_join_event, prepare_join_event},
        },
    },
    events::{
        StateEventType,
        room::{
            join_rules::RoomJoinRulesEventContent,
            member::{MembershipState, RoomMemberEventContent},
        },
    },
    room::{AllowRule, JoinRule},
};
use serde_json::value::{RawValue as RawJsonValue, to_raw_value};

use super::{Service, merge_member_content, outgoing_pdu, sender_servers, supported_room_versions};
use crate::{
    federation::{Candidates, WhenAllBackedOff},
    rooms::{
        state::RoomMutexGuard,
        state_compressor::{CompressedState, HashSetCompressStateEvent},
    },
};

const INCOMPATIBLE_ROOM_VERSION_LIMIT: usize = 15;

#[derive(Debug)]
pub struct Join<'a> {
    pub sender_user: &'a UserId,
    pub room_id: &'a RoomId,
    pub orig_room_id: Option<&'a RoomOrAliasId>,
    pub reason: Option<String>,
    pub servers: &'a [OwnedServerName],
    pub is_appservice: bool,
    pub extra_content: Option<CanonicalJsonObject>,
}

type StateIds = HashMap<u64, OwnedEventId>;

#[implement(Service)]
#[tracing::instrument(name = "join", level = "debug", skip_all, fields(%sender_user, %room_id))]
pub async fn join<'a>(
    &'a self,
    Join {
        sender_user,
        room_id,
        orig_room_id,
        reason,
        servers,
        is_appservice,
        extra_content,
    }: Join<'a>,
) -> Result {
    let servers = self
        .servers_for_room(sender_user, room_id, orig_room_id, servers)
        .await;

    let (federation_lock, state_lock) = self.lock_join(room_id, &servers).await;

    let user_is_guest = !is_appservice
        && self
            .services
            .users
            .is_deactivated(sender_user)
            .await
            .unwrap_or(false);

    if user_is_guest && !self.services.state_accessor.guest_can_join(room_id).await {
        return Err!(Request(Forbidden(
            "Guests are not allowed to join this room"
        )));
    }

    if self
        .services
        .state_cache
        .is_joined(sender_user, room_id)
        .await
    {
        debug_warn!("{sender_user} is already joined in {room_id}");
        return Ok(());
    }

    if let Ok(membership) = self
        .services
        .state_accessor
        .get_member(room_id, sender_user)
        .await
        && membership.membership == MembershipState::Ban
        && !self
            .services
            .state_cache
            .is_invited(sender_user, room_id)
            .await
    {
        debug_warn!("{sender_user} is banned from {room_id} but attempted to join");
        return Err!(Request(Forbidden("You are banned from the room.")));
    }

    match federation_lock {
        Some(federation_lock) if !self.is_local_join(room_id, &servers).await => {
            self.join_remote(
                sender_user,
                room_id,
                reason,
                &servers,
                federation_lock,
                state_lock,
                extra_content,
            )
            .boxed()
            .await
        }
        federation_lock => {
            drop(federation_lock);

            self.join_local(
                sender_user,
                room_id,
                reason,
                &servers,
                state_lock,
                extra_content,
            )
            .boxed()
            .await
        }
    }
}

#[implement(Service)]
async fn lock_join(
    &self,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> (Option<RoomMutexGuard>, RoomMutexGuard) {
    if !self.is_local_join(room_id, servers).await {
        let (federation_lock, state_lock) = self.lock_join_remote(room_id).await;

        return (Some(federation_lock), state_lock);
    }

    let state_lock = self.services.state.mutex.lock(room_id).await;

    if self.is_local_join(room_id, servers).await {
        return (None, state_lock);
    }

    drop(state_lock);

    let (federation_lock, state_lock) = self.lock_join_remote(room_id).await;

    (Some(federation_lock), state_lock)
}

#[implement(Service)]
async fn is_local_join(&self, room_id: &RoomId, servers: &[OwnedServerName]) -> bool {
    self.is_local_only(servers)
        || self
            .services
            .state_cache
            .server_in_room(self.services.server_state.server_name(), room_id)
            .await
}

#[implement(Service)]
async fn lock_join_remote(&self, room_id: &RoomId) -> (RoomMutexGuard, RoomMutexGuard) {
    let federation_lock = self
        .services
        .event_handler
        .mutex_federation
        .lock(room_id)
        .await;

    let state_lock = self.services.state.mutex.lock(room_id).await;

    (federation_lock, state_lock)
}

#[implement(Service)]
#[expect(clippy::too_many_arguments)]
#[tracing::instrument(name = "remote", level = "debug", skip_all, fields(?servers))]
async fn join_remote(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    _federation_lock: RoomMutexGuard,
    state_lock: RoomMutexGuard,
    extra_content: Option<CanonicalJsonObject>,
) -> Result {
    info!("Joining {room_id} over federation.");

    let (make_join_response, remote_server) = self
        .make_join_request(sender_user, room_id, servers)
        .await?;

    let room_version_id = self.require_supported_remote_room_version(&make_join_response)?;

    let (mut join_event, event_id, join_authorized_via_users_server) = self
        .create_join_event(
            room_id,
            sender_user,
            &make_join_response.event,
            &room_version_id,
            reason,
            extra_content,
        )
        .await?;

    let mut response = self
        .execute_send_join(
            &remote_server,
            room_id,
            &event_id,
            join_event.clone(),
            &room_version_id,
        )
        .await?;

    if response.members_omitted {
        self.fetch_omitted_state(&remote_server, room_id, &event_id, servers, &mut response)
            .await?;
    }

    if join_authorized_via_users_server.is_some() {
        merge_restricted_signature(
            &remote_server,
            &event_id,
            &room_version_id,
            &response,
            &mut join_event,
        )?;
    }

    let shortroomid = self.services.short.get_or_create_shortroomid(room_id).await;

    info!(%room_id, %shortroomid, "Initialized room. Parsing join event...");

    let parsed_join_pdu = parse_federation_pdu(room_id, &event_id, join_event.clone())?;

    info!(
        events = response
            .state
            .len()
            .saturating_add(response.auth_chain.len()),
        "Acquiring server signing keys for response events..."
    );

    self.services
        .server_keys
        .acquire_events_pubkeys(response.auth_chain.iter().chain(response.state.iter()))
        .await;

    let state = self
        .ingest_send_join_state(room_id, &room_version_id, &response.state)
        .await;

    self.ingest_send_join_auth_chain(room_id, &room_version_id, &response.auth_chain)
        .await;

    debug!("Running send_join auth check...");
    self.auth_check_join(&room_version_id, &parsed_join_pdu, &state)
        .await?;

    self.apply_state(room_id, &state, &state_lock).await?;

    self.services.state_cache.update_joined_count(room_id).await;

    let statehash_after_join = self
        .services
        .state
        .append_to_state(&parsed_join_pdu)
        .await?;

    info!(event_id = %parsed_join_pdu.event_id, "Appending new room join event...");

    self.services
        .timeline
        .append_pdu(
            &parsed_join_pdu,
            join_event,
            once(parsed_join_pdu.event_id.borrow()),
            &state_lock,
        )
        .await?;

    self.services
        .state
        .set_room_state(room_id, statehash_after_join, &state_lock);

    info!(statehash = %statehash_after_join, "Set final room state for new room.");

    Ok(())
}

#[implement(Service)]
fn require_supported_remote_room_version(
    &self,
    make_join_response: &prepare_join_event::v1::Response,
) -> Result<RoomVersionId> {
    let Some(room_version_id) = make_join_response.room_version.clone() else {
        return Err!(BadServerResponse(
            "Remote room version is not supported by phantom"
        ));
    };

    if !self.supported_room_version(&room_version_id) {
        return Err!(BadServerResponse(
            "Remote room version {room_version_id} is not supported by phantom"
        ));
    }

    Ok(room_version_id)
}

#[implement(Service)]
async fn execute_send_join(
    &self,
    remote_server: &OwnedServerName,
    room_id: &RoomId,
    event_id: &OwnedEventId,
    join_event: CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> Result<create_join_event::v2::RoomState> {
    let mut request = create_join_event::v2::Request::new(
        room_id.to_owned(),
        event_id.clone(),
        outgoing_pdu(join_event, room_version_id),
    );

    request.omit_members = true;

    info!("Asking {remote_server} for fast_join in room {room_id}");

    let response = self
        .services
        .federation
        .execute(remote_server, request)
        .await
        .inspect_err(|e| error!("send_join failed: {e}"))?
        .room_state;

    info!(
        fast_join = response.members_omitted,
        auth_chain = response.auth_chain.len(),
        state = response.state.len(),
        servers = response.servers_in_room.as_ref().map_or(0, Vec::len),
        "send_join finished"
    );

    Ok(response)
}

#[implement(Service)]
async fn fetch_omitted_state(
    &self,
    remote_server: &OwnedServerName,
    room_id: &RoomId,
    event_id: &OwnedEventId,
    servers: &[OwnedServerName],
    response: &mut create_join_event::v2::RoomState,
) -> Result {
    let eligible =
        self.omitted_state_servers(remote_server, servers, response.servers_in_room.as_deref());

    let candidates = self
        .services
        .federation
        .rank_candidates(eligible, WhenAllBackedOff::Attempt)
        .await;

    let mut last_error = Err!(BadServerResponse(
        "No server provided omitted send_join state."
    ));

    for server in candidates {
        info!("Asking {server} for state in room {room_id}");

        let request = get_room_state::v1::Request::new(event_id.clone(), room_id.to_owned());

        match self.services.federation.execute(&server, request).await {
            Err(e) => {
                debug_warn!(?server, "state fetch failed: {e}");
                last_error = Err(e);
            }
            Ok(mut state) => {
                response.auth_chain = take(&mut state.auth_chain);
                response.state = take(&mut state.pdus);

                info!(
                    auth_chain = response.auth_chain.len(),
                    state = response.state.len(),
                    "state finished"
                );

                return Ok(());
            }
        }
    }

    last_error
}

#[implement(Service)]
fn omitted_state_servers(
    &self,
    remote_server: &OwnedServerName,
    servers: &[OwnedServerName],
    servers_in_room: Option<&[String]>,
) -> Candidates {
    let extracted = servers_in_room
        .into_iter()
        .flatten()
        .filter_map(|server| OwnedServerName::try_from(server.as_str()).ok());

    let mut seen = BTreeSet::new();

    once(remote_server.clone())
        .chain(extracted)
        .chain(servers.iter().cloned())
        .filter(|server| !self.services.server_state.server_is_ours(server))
        .filter(move |server| seen.insert(server.clone()))
        .take(
            self.services
                .server
                .config
                .membership
                .max_make_join_attempts_per_join_attempt,
        )
        .collect()
}

fn merge_restricted_signature(
    remote_server: &OwnedServerName,
    event_id: &OwnedEventId,
    room_version_id: &RoomVersionId,
    response: &create_join_event::v2::RoomState,
    join_event: &mut CanonicalJsonObject,
) -> Result {
    let Some(signed_raw) = &response.event else {
        return Ok(());
    };

    debug_info!(
        "There is a signed event with join_authorized_via_users_server. This room is probably \
         using restricted joins. Adding signature to our event"
    );

    let (signed_event_id, signed_value) = gen_event_id_canonical_json(signed_raw, room_version_id)
        .map_err(|e| {
            err!(Request(BadJson(warn!(
                "Could not convert event to canonical JSON: {e}"
            ))))
        })?;

    if signed_event_id != *event_id {
        return Err!(Request(BadJson(warn!(
            "Server {remote_server} sent event {signed_event_id} when {event_id} was expected"
        ))));
    }

    let signature = signed_value
        .get("signatures")
        .and_then(CanonicalJsonValue::as_object)
        .ok_or_else(|| {
            err!(BadServerResponse(warn!(
                "Server {remote_server} sent invalid signatures type"
            )))
        })
        .and_then(|signatures| {
            signatures.get(remote_server.as_str()).ok_or_else(|| {
                err!(BadServerResponse(warn!(
                    "Server {remote_server} did not send its signature for a restricted room"
                )))
            })
        });

    match signature {
        Ok(signature) => {
            join_event
                .get_mut("signatures")
                .and_then(CanonicalJsonValue::as_object_mut)
                .expect("we created a valid pdu")
                .insert(remote_server.as_str().into(), signature.clone());
        }
        Err(e) => {
            warn!(
                "Server {remote_server} sent invalid signature in send_join signatures for event \
                 {signed_value:?}: {e:?}",
            );
        }
    }

    Ok(())
}

fn parse_federation_pdu(
    room_id: &RoomId,
    event_id: &OwnedEventId,
    value: CanonicalJsonObject,
) -> Result<PduEvent> {
    let pdu = PduEvent::from_id_val(event_id, value).map_err(|e| {
        err!(BadServerResponse(
            "Invalid PDU {event_id} in response: {e:?}"
        ))
    })?;

    if pdu.room_id != room_id {
        return Err!(BadServerResponse(
            "PDU {event_id} belongs to {} rather than {room_id}",
            pdu.room_id
        ));
    }

    Ok(pdu)
}

#[implement(Service)]
async fn ingest_send_join_state(
    &self,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    state_pdus: &[Box<RawJsonValue>],
) -> StateIds {
    info!(
        events = state_pdus.len(),
        "Going through send_join response room_state..."
    );

    let cork = self.services.db.engine.cork_and_flush();

    let mut state = StateIds::new();

    for pdu in state_pdus {
        let (event_id, value) = match self
            .services
            .server_keys
            .validate_and_add_event_id_no_fetch(pdu, room_version_id)
            .await
        {
            Ok(validated) => validated,
            Err(e) => {
                debug_error!("Invalid send_join state event: {e:?}");
                continue;
            }
        };

        let parsed = match parse_federation_pdu(room_id, &event_id, value.clone()) {
            Ok(parsed) => parsed,
            Err(e) => {
                debug_warn!("Invalid PDU {event_id:?} in send_join response: {e:?}");
                continue;
            }
        };

        self.services.outlier.add_pdu_outlier(&event_id, &value);

        if let Some(state_key) = &parsed.state_key {
            let shortstatekey = self
                .services
                .short
                .get_or_create_shortstatekey(&parsed.kind.to_string().into(), state_key)
                .await;

            state.insert(shortstatekey, parsed.event_id);
        }
    }

    drop(cork);

    state
}

#[implement(Service)]
async fn ingest_send_join_auth_chain(
    &self,
    room_id: &RoomId,
    room_version_id: &RoomVersionId,
    auth_chain: &[Box<RawJsonValue>],
) {
    info!(
        events = auth_chain.len(),
        "Going through send_join response auth_chain..."
    );

    let requires_room_id = room_version_id
        .rules()
        .is_none_or(|rules| rules.event_format.require_room_create_room_id);

    let cork = self.services.db.engine.cork_and_flush();

    for pdu in auth_chain {
        let (event_id, mut value) = match self
            .services
            .server_keys
            .validate_and_add_event_id_no_fetch(pdu, room_version_id)
            .await
        {
            Ok(validated) => validated,
            Err(e) => {
                debug_error!("Invalid send_join auth_chain event: {e:?}");
                continue;
            }
        };

        let is_create =
            value.get("type").and_then(CanonicalJsonValue::as_str) == Some("m.room.create");

        if !requires_room_id && is_create {
            value.insert(
                "room_id".into(),
                CanonicalJsonValue::String(room_id.as_str().into()),
            );
        }

        self.services.outlier.add_pdu_outlier(&event_id, &value);
    }

    drop(cork);
}

#[implement(Service)]
async fn auth_check_join(
    &self,
    room_version_id: &RoomVersionId,
    join_pdu: &PduEvent,
    state: &StateIds,
) -> Result {
    let room_version = RoomVersion::new(room_version_id)?;

    let fetch = |event_type: &StateEventType, state_key: &str| {
        let event_type = event_type.clone();
        let state_key = state_key.to_owned();

        async move {
            let shortstatekey = self
                .services
                .short
                .get_shortstatekey(&event_type, &state_key)
                .await
                .ok()?;

            let event_id = state.get(&shortstatekey)?;

            self.services.timeline.get_pdu(event_id).await.ok()
        }
    };

    let authorized = state_res::auth_check(&room_version, join_pdu, None, fetch)
        .await
        .map_err(|e| {
            err!(Request(Forbidden(error!(
                "send_join auth check failed: {e:?}"
            ))))
        })?;

    if !authorized {
        return Err!(Request(Forbidden(
            "Join event failed the send_join auth check."
        )));
    }

    Ok(())
}

#[implement(Service)]
pub(super) async fn apply_state(
    &self,
    room_id: &RoomId,
    state: &StateIds,
    state_lock: &RoomMutexGuard,
) -> Result {
    info!(events = state.len(), "Compressing state...");

    let compressed: CompressedState = self
        .services
        .state_compressor
        .compress_state_events(
            state
                .iter()
                .map(|(shortstatekey, event_id)| (shortstatekey, event_id.borrow())),
        )
        .collect()
        .await;

    debug!("Saving compressed state...");

    let HashSetCompressStateEvent {
        shortstatehash,
        added,
        removed,
    } = self
        .services
        .state_compressor
        .save_state(room_id, Arc::new(compressed))
        .await?;

    debug!(state_hash = ?shortstatehash, "Forcing state for new room...");

    self.services
        .state
        .force_state(room_id, shortstatehash, added, removed, state_lock)
        .await
}

#[implement(Service)]
#[tracing::instrument(name = "local", level = "debug", skip_all)]
async fn join_local(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    state_lock: RoomMutexGuard,
    extra_content: Option<CanonicalJsonObject>,
) -> Result {
    debug_info!("We can join locally");

    let restriction_rooms = self.restriction_rooms(room_id).await;

    let is_joined_restricted_rooms = stream::iter(&restriction_rooms)
        .any(|restriction_room_id| {
            self.services
                .state_cache
                .is_joined(sender_user, restriction_room_id)
        })
        .await;

    let join_authorized_via_users_server = if is_joined_restricted_rooms {
        self.authorizing_user(room_id, sender_user, &state_lock)
            .await
    } else {
        None
    };

    let mut content = RoomMemberEventContent::new(MembershipState::Join);
    content.reason = reason.clone();
    content.join_authorized_via_users_server = join_authorized_via_users_server;

    self.services
        .users
        .fill_profile_data(sender_user, &mut content)
        .await;

    let content = merge_member_content(content, extra_content.as_ref())?;

    let pdu_builder = PduBuilder {
        event_type: StateEventType::RoomMember.into(),
        content: to_raw_value(&content)?,
        state_key: Some(sender_user.as_str().into()),
        ..PduBuilder::default()
    };

    let Err(error) = self
        .services
        .timeline
        .build_and_append_pdu(pdu_builder, sender_user, room_id, &state_lock)
        .await
    else {
        return Ok(());
    };

    if restriction_rooms.is_empty() && self.is_local_only(servers) {
        return Err(error);
    }

    warn!(
        "We couldn't do the join locally, maybe federation can help to satisfy the restricted \
         join requirements"
    );

    drop(state_lock);

    let Ok((make_join_response, remote_server)) =
        self.make_join_request(sender_user, room_id, servers).await
    else {
        return Err(error);
    };

    let room_version_id = self.require_supported_remote_room_version(&make_join_response)?;

    let (join_event, event_id, _) = self
        .create_join_event(
            room_id,
            sender_user,
            &make_join_response.event,
            &room_version_id,
            reason,
            extra_content,
        )
        .await?;

    let send_join_response = self
        .execute_send_join(
            &remote_server,
            room_id,
            &event_id,
            join_event,
            &room_version_id,
        )
        .await?;

    let Some(signed_raw) = send_join_response.event else {
        return Err(error);
    };

    let (signed_event_id, signed_value) =
        gen_event_id_canonical_json(&signed_raw, &room_version_id).map_err(|e| {
            err!(Request(BadJson(warn!(
                "Could not convert event to canonical JSON: {e}"
            ))))
        })?;

    if signed_event_id != event_id {
        return Err!(Request(BadJson(warn!(
            "Server {remote_server} sent event {signed_event_id} when {event_id} was expected"
        ))));
    }

    self.services
        .event_handler
        .handle_incoming_pdu(
            &remote_server,
            room_id,
            &signed_event_id,
            signed_value,
            true,
        )
        .await?
        .ok_or_else(|| {
            err!(Request(InvalidParam(
                "Signed join was not accepted as a timeline event."
            )))
        })?;

    Ok(())
}

#[implement(Service)]
async fn restriction_rooms(&self, room_id: &RoomId) -> Vec<ruma::OwnedRoomId> {
    let Ok(join_rules) = self
        .services
        .state_accessor
        .room_state_get_content::<RoomJoinRulesEventContent>(
            room_id,
            &StateEventType::RoomJoinRules,
            "",
        )
        .await
    else {
        return Vec::new();
    };

    match join_rules.join_rule {
        JoinRule::Restricted(restricted) | JoinRule::KnockRestricted(restricted) => restricted
            .allow
            .into_iter()
            .filter_map(|rule| match rule {
                AllowRule::RoomMembership(membership) => Some(membership.room_id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[implement(Service)]
async fn authorizing_user(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    state_lock: &RoomMutexGuard,
) -> Option<OwnedUserId> {
    let local_users: Vec<OwnedUserId> = self
        .services
        .state_cache
        .local_users_in_room(room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    for user in local_users {
        if self
            .services
            .state_accessor
            .user_can_invite(room_id, &user, sender_user, state_lock)
            .await
        {
            return Some(user);
        }
    }

    None
}

#[implement(Service)]
#[tracing::instrument(name = "make_join", level = "debug", skip_all)]
async fn create_join_event(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    join_event_stub: &RawJsonValue,
    room_version_id: &RoomVersionId,
    reason: Option<String>,
    extra_content: Option<CanonicalJsonObject>,
) -> Result<(CanonicalJsonObject, OwnedEventId, Option<OwnedUserId>)> {
    let mut event: CanonicalJsonObject =
        serde_json::from_str(join_event_stub.get()).map_err(|e| {
            err!(BadServerResponse(
                "Invalid make_join event json received from server: {e:?}"
            ))
        })?;

    let restricted_join_rule = room_version_id
        .rules()
        .is_some_and(|rules| rules.authorization.restricted_join_rule);

    let join_authorized_via_users_server = restricted_join_rule
        .then(|| event.get("content"))
        .flatten()
        .and_then(CanonicalJsonValue::as_object)
        .and_then(|content| content.get("join_authorised_via_users_server"))
        .and_then(CanonicalJsonValue::as_str)
        .and_then(|user| OwnedUserId::try_from(user).ok());

    let mut content = RoomMemberEventContent::new(MembershipState::Join);
    content.reason = reason;
    content.join_authorized_via_users_server = join_authorized_via_users_server.clone();

    self.services
        .users
        .fill_profile_data(sender_user, &mut content)
        .await;

    let content = merge_member_content(content, extra_content.as_ref())?;

    self.complete_member_event(&mut event, room_id, sender_user, content)?;

    let event_id = self
        .services
        .server_keys
        .gen_id_hash_and_sign_event(&mut event, room_version_id)?;

    Ok((event, event_id, join_authorized_via_users_server))
}

#[implement(Service)]
#[tracing::instrument(name = "make_join", level = "debug", skip_all, fields(?servers))]
async fn make_join_request(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> Result<(prepare_join_event::v1::Response, OwnedServerName)> {
    let max_attempts = self
        .services
        .server
        .config
        .membership
        .max_make_join_attempts_per_join_attempt;

    let mut attempts: usize = 0;
    let mut incompatible: usize = 0;
    let mut last_error = Err!(BadServerResponse(
        "No server available to assist in joining."
    ));

    for remote_server in servers {
        if self.services.server_state.server_is_ours(remote_server) {
            continue;
        }

        info!("Asking {remote_server} for make_join ({attempts})");

        let mut request =
            prepare_join_event::v1::Request::new(room_id.to_owned(), sender_user.to_owned());
        request.ver = supported_room_versions();

        let response = self
            .services
            .federation
            .execute(remote_server, request)
            .await;

        trace!("make_join response: {response:?}");
        attempts = attempts.saturating_add(1);

        let e = match response {
            Ok(response) => return Ok((response, remote_server.clone())),
            Err(e) => e,
        };

        if matches!(
            e.kind(),
            ErrorKind::IncompatibleRoomVersion(_) | ErrorKind::UnsupportedRoomVersion
        ) {
            incompatible = incompatible.saturating_add(1);
        }

        if incompatible > INCOMPATIBLE_ROOM_VERSION_LIMIT {
            info!(
                "{INCOMPATIBLE_ROOM_VERSION_LIMIT} servers have responded with \
                 M_INCOMPATIBLE_ROOM_VERSION or M_UNSUPPORTED_ROOM_VERSION, assuming that phantom \
                 does not support the room version {room_id}: {e}"
            );

            return Err!(BadServerResponse(
                "Room version is not supported by phantom"
            ));
        }

        if attempts >= max_attempts {
            warn!(?remote_server, "last make_join failure reason: {e}");
            warn!(
                "{max_attempts} servers failed to provide valid make_join response, assuming no \
                 server can assist in joining."
            );

            return Err!(BadServerResponse(
                "No server available to assist in joining."
            ));
        }

        last_error = Err(e);
    }

    last_error
}

#[implement(Service)]
pub(super) async fn servers_for_room(
    &self,
    user_id: &UserId,
    room_id: &RoomId,
    orig_room_id: Option<&RoomOrAliasId>,
    via: &[OwnedServerName],
) -> Vec<OwnedServerName> {
    let state_cache = &self.services.state_cache;

    let mut additional_servers: Vec<OwnedServerName> = state_cache
        .servers_invite_via(room_id)
        .map(ToOwned::to_owned)
        .collect()
        .await;

    if let Ok(invite_state) = state_cache.invite_state(user_id, room_id).await {
        additional_servers.extend(sender_servers(&invite_state));
    }

    let mut servers = via.to_vec();
    shuffle(&mut servers);

    let has_remote_via = via
        .iter()
        .any(|server| !self.services.server_state.server_is_ours(server));

    if !has_remote_via {
        if let Some(server_name) = room_id.server_name() {
            servers.insert(0, server_name.to_owned());
        }

        if let Some(orig_server_name) = orig_room_id.and_then(RoomOrAliasId::server_name) {
            servers.insert(0, orig_server_name.to_owned());
        }
    }

    shuffle(&mut additional_servers);
    servers.extend(additional_servers);

    order_servers(
        servers,
        &self
            .services
            .server
            .config
            .membership
            .deprioritize_joins_through_servers,
    )
}

fn order_servers(
    servers: Vec<OwnedServerName>,
    deprioritized: &regex::RegexSet,
) -> Vec<OwnedServerName> {
    let mut seen = HashSet::new();

    let (mut preferred, demoted): (Vec<_>, Vec<_>) = servers
        .into_iter()
        .filter(|server| seen.insert(server.clone()))
        .partition(|server| !deprioritized.is_match(server.host()));

    preferred.extend(demoted);

    debug_info!(?preferred);

    preferred
}

#[cfg(test)]
mod tests {
    use regex::RegexSet;
    use ruma::OwnedServerName;

    use super::order_servers;

    fn names(servers: &[OwnedServerName]) -> Vec<&str> {
        servers.iter().map(AsRef::as_ref).collect()
    }

    #[test]
    fn duplicates_keep_their_first_position() {
        let servers = ["a.test", "b.test", "a.test", "c.test", "b.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let ordered = order_servers(servers.to_vec(), &RegexSet::empty());

        assert_eq!(names(&ordered), ["a.test", "b.test", "c.test"]);
    }

    #[test]
    fn deprioritized_servers_move_to_the_back_in_order() {
        let servers = ["matrix.org", "a.test", "sub.matrix.org", "b.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let deprioritized = RegexSet::new([r"matrix\.org"]).expect("valid");
        let ordered = order_servers(servers.to_vec(), &deprioritized);

        assert_eq!(
            names(&ordered),
            ["a.test", "b.test", "matrix.org", "sub.matrix.org"]
        );
    }

    #[test]
    fn adjacent_deprioritized_servers_are_all_moved() {
        let servers = ["matrix.org", "sub.matrix.org", "a.test"]
            .map(|name| OwnedServerName::try_from(name).expect("valid"));

        let deprioritized = RegexSet::new([r"matrix\.org"]).expect("valid");
        let ordered = order_servers(servers.to_vec(), &deprioritized);

        assert_eq!(names(&ordered), ["a.test", "matrix.org", "sub.matrix.org"]);
    }
}
