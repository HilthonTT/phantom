//! The federated join: `make_join`, `send_join`, and the state they return.

use std::{borrow::Borrow, collections::BTreeSet, iter::once, mem::take};

use phantom_core::{
    Err, Result, debug, debug_info, debug_warn, err, error, implement, info,
    matrix::{PduEvent, pdu::gen_event_id_canonical_json},
    warn,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedServerName, RoomId, RoomVersionId,
    UserId,
    api::federation::{
        event::get_room_state,
        membership::{create_join_event, prepare_join_event},
    },
};

use crate::{
    federation::{Candidates, WhenAllBackedOff},
    membership::{Service, outgoing_pdu},
    rooms::state::RoomMutexGuard,
};

#[implement(Service)]
#[expect(clippy::too_many_arguments)]
#[tracing::instrument(name = "remote", level = "debug", skip_all, fields(?servers))]
pub(super) async fn join_remote(
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
pub(super) fn require_supported_remote_room_version(
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
pub(super) async fn execute_send_join(
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

pub(super) fn parse_federation_pdu(
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
