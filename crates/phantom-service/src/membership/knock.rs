use std::{borrow::Borrow, collections::HashMap, iter::once};

use futures::FutureExt;
use phantom_core::{
    Err, Result, debug_info, debug_warn, err, implement, info,
    matrix::{PduBuilder, PduEvent, pdu::gen_event_id},
    trace, warn,
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedServerName, RoomId, RoomOrAliasId,
    RoomVersionId, UserId,
    api::federation::membership::{RawStrippedState, create_knock_event, prepare_knock_event},
    canonical_json::to_canonical_value,
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};

use super::{
    Service, StrippedCreateVerdict, dedup_stripped_state, enforce_stripped_create,
    into_client_stripped, outgoing_pdu, supported_room_versions, v12_room_ids,
};
use crate::rooms::state::RoomMutexGuard;

const MAKE_KNOCK_ATTEMPTS: usize = 40;

#[implement(Service)]
#[tracing::instrument(name = "knock", level = "debug", skip_all, fields(%sender_user, %room_id))]
pub async fn knock<'a>(
    &'a self,
    sender_user: &'a UserId,
    room_id: &'a RoomId,
    orig_server_name: Option<&'a RoomOrAliasId>,
    reason: Option<String>,
    servers: &'a [OwnedServerName],
    state_lock: &'a RoomMutexGuard,
) -> Result {
    let servers = self
        .servers_for_room(sender_user, room_id, orig_server_name, servers)
        .await;

    let state_cache = &self.services.state_cache;

    if state_cache.is_invited(sender_user, room_id).await {
        debug_warn!("{sender_user} is already invited in {room_id} but attempted to knock");
        return Err!(Request(Forbidden(
            "You cannot knock on a room you are already invited/accepted to."
        )));
    }

    if state_cache.is_joined(sender_user, room_id).await {
        debug_warn!("{sender_user} is already joined in {room_id} but attempted to knock");
        return Err!(Request(Forbidden(
            "You cannot knock on a room you are already joined in."
        )));
    }

    let server_in_room = state_cache
        .server_in_room(self.services.server_state.server_name(), room_id)
        .await;

    if server_in_room && state_cache.is_knocked(sender_user, room_id).await {
        debug_warn!("{sender_user} is already knocked in {room_id}");
        return Ok(());
    }

    if let Ok(membership) = self
        .services
        .state_accessor
        .get_member(room_id, sender_user)
        .await
        && membership.membership == MembershipState::Ban
    {
        debug_warn!("{sender_user} is banned from {room_id} but attempted to knock");
        return Err!(Request(Forbidden(
            "You cannot knock on a room you are banned from."
        )));
    }

    if server_in_room || self.is_local_only(&servers) {
        self.knock_local(sender_user, room_id, reason, &servers, state_lock)
            .boxed()
            .await
    } else {
        self.knock_remote(sender_user, room_id, reason, &servers, state_lock)
            .boxed()
            .await
    }
}

#[implement(Service)]
async fn knock_content(
    &self,
    sender_user: &UserId,
    reason: Option<String>,
) -> RoomMemberEventContent {
    let mut content = RoomMemberEventContent::new(MembershipState::Knock);
    content.reason = reason;

    self.services
        .profile
        .fill_profile_data(sender_user, &mut content)
        .await;

    content
}

#[implement(Service)]
async fn knock_local(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    state_lock: &RoomMutexGuard,
) -> Result {
    debug_info!("We can knock locally");

    let room_version_id = self.services.state.get_room_version(room_id).await?;

    ensure_room_version_supports_knock(&room_version_id)?;

    let content = self.knock_content(sender_user, reason.clone()).await;

    let Err(error) = self
        .services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(sender_user.to_string(), &content),
            sender_user,
            room_id,
            state_lock,
        )
        .await
    else {
        return Ok(());
    };

    if self.is_local_only(servers) {
        return Err(error);
    }

    warn!("We couldn't do the knock locally, maybe federation can help to satisfy the knock");

    let (knock_event, event_id, knock_room_state, _) = self
        .knock_over_federation(sender_user, room_id, reason, servers)
        .await?;

    self.finalize_knock(
        room_id,
        sender_user,
        &event_id,
        knock_event,
        knock_room_state,
        state_lock,
    )
    .await
}

fn ensure_room_version_supports_knock(room_version_id: &RoomVersionId) -> Result {
    let knocking = room_version_id
        .rules()
        .is_some_and(|rules| rules.authorization.knocking);

    if !knocking {
        return Err!(Request(Forbidden("This room does not support knocking.")));
    }

    Ok(())
}

#[implement(Service)]
async fn knock_over_federation(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
) -> Result<(
    CanonicalJsonObject,
    OwnedEventId,
    Vec<RawStrippedState>,
    RoomVersionId,
)> {
    let (make_knock_response, remote_server) = self
        .make_knock_request(sender_user, room_id, servers)
        .await?;

    info!("make_knock finished");

    let room_version_id = make_knock_response.room_version.clone();

    if !self.supported_room_version(&room_version_id) {
        return Err!(BadServerResponse(
            "Remote room version {room_version_id} is not supported by phantom"
        ));
    }

    let (knock_event, event_id) = self
        .build_knock_event(
            sender_user,
            room_id,
            reason,
            &make_knock_response,
            &room_version_id,
        )
        .await?;

    let knock_room_state = self
        .execute_send_knock(
            &remote_server,
            room_id,
            &event_id,
            &knock_event,
            &room_version_id,
        )
        .await?;

    self.services.short.get_or_create_shortroomid(room_id).await;

    Ok((knock_event, event_id, knock_room_state, room_version_id))
}

#[implement(Service)]
async fn finalize_knock(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    event_id: &OwnedEventId,
    knock_event: CanonicalJsonObject,
    knock_room_state: Vec<RawStrippedState>,
    state_lock: &RoomMutexGuard,
) -> Result {
    let parsed_knock_pdu = self
        .record_knock(
            room_id,
            sender_user,
            event_id,
            &knock_event,
            knock_room_state,
        )
        .await?;

    info!("Appending room knock event locally");

    self.services
        .timeline
        .append_pdu(
            &parsed_knock_pdu,
            knock_event,
            once(parsed_knock_pdu.event_id.borrow()),
            state_lock,
        )
        .await?;

    Ok(())
}

#[implement(Service)]
async fn record_knock(
    &self,
    room_id: &RoomId,
    sender_user: &UserId,
    event_id: &OwnedEventId,
    knock_event: &CanonicalJsonObject,
    knock_room_state: Vec<RawStrippedState>,
) -> Result<PduEvent> {
    info!("Parsing knock event");

    let parsed_knock_pdu = PduEvent::from_id_val(event_id, knock_event.clone())
        .map_err(|e| err!(BadServerResponse("Invalid knock event PDU: {e:?}")))?;

    info!("Updating membership locally to knock state with provided stripped state events");

    let membership_event = parsed_knock_pdu.get_content::<RoomMemberEventContent>()?;

    let last_state = knock_room_state
        .into_iter()
        .filter_map(|state| into_client_stripped(room_id, state))
        .collect();

    self.services
        .state_cache
        .update_membership(
            room_id,
            sender_user,
            membership_event,
            sender_user,
            Some(last_state),
            None,
            false,
        )
        .await?;

    Ok(parsed_knock_pdu)
}

#[implement(Service)]
async fn knock_remote(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    servers: &[OwnedServerName],
    state_lock: &RoomMutexGuard,
) -> Result {
    info!("Knocking {room_id} over federation.");

    let (knock_event, event_id, knock_room_state, room_version_id) = self
        .knock_over_federation(sender_user, room_id, reason, servers)
        .await?;

    let state_map = self
        .ingest_send_knock_state(room_id, &knock_room_state, &room_version_id)
        .await?;

    self.apply_state(room_id, &state_map, state_lock).await?;

    let parsed_knock_pdu = PduEvent::from_id_val(&event_id, knock_event.clone())
        .map_err(|e| err!(BadServerResponse("Invalid knock event PDU: {e:?}")))?;

    let statehash_after_knock = self
        .services
        .state
        .append_to_state(&parsed_knock_pdu)
        .await?;

    self.finalize_knock(
        room_id,
        sender_user,
        &event_id,
        knock_event,
        knock_room_state,
        state_lock,
    )
    .await?;

    info!("Setting final room state for new room");

    self.services
        .state
        .set_room_state(room_id, statehash_after_knock, state_lock);

    Ok(())
}

#[implement(Service)]
async fn build_knock_event(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    reason: Option<String>,
    make_knock_response: &prepare_knock_event::v1::Response,
    room_version_id: &RoomVersionId,
) -> Result<(CanonicalJsonObject, OwnedEventId)> {
    let mut knock_event: CanonicalJsonObject =
        serde_json::from_str(make_knock_response.event.get()).map_err(|e| {
            err!(BadServerResponse(
                "Invalid make_knock event json received from server: {e:?}"
            ))
        })?;

    let content = self.knock_content(sender_user, reason).await;

    self.complete_member_event(
        &mut knock_event,
        room_id,
        sender_user,
        to_canonical_value(content)?,
    )?;

    self.services
        .server_keys
        .hash_and_sign_event(&mut knock_event, room_version_id)?;

    let event_id = gen_event_id(&knock_event, room_version_id)?;

    knock_event.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.as_str().into()),
    );

    Ok((knock_event, event_id))
}

#[implement(Service)]
async fn execute_send_knock(
    &self,
    remote_server: &OwnedServerName,
    room_id: &RoomId,
    event_id: &OwnedEventId,
    knock_event: &CanonicalJsonObject,
    room_version_id: &RoomVersionId,
) -> Result<Vec<RawStrippedState>> {
    info!("Asking {remote_server} for send_knock in room {room_id}");

    let request = create_knock_event::v1::Request::new(
        room_id.to_owned(),
        event_id.clone(),
        outgoing_pdu(knock_event.clone(), room_version_id),
    );

    let response = self
        .services
        .federation
        .execute(remote_server, request)
        .await?;

    info!("send_knock finished");

    Ok(dedup_stripped_state(response.knock_room_state))
}

#[implement(Service)]
#[expect(deprecated)]
async fn ingest_send_knock_state(
    &self,
    room_id: &RoomId,
    knock_room_state: &[RawStrippedState],
    room_version_id: &RoomVersionId,
) -> Result<HashMap<u64, OwnedEventId>> {
    info!("Going through send_knock response knock state events");

    let verdict = self
        .validate_stripped_create(knock_room_state, room_id, room_version_id)
        .await?;

    let enforce = self
        .services
        .server
        .config
        .membership
        .enforce_stripped_state_pdu_validation;

    let drop_create = enforce_stripped_create(verdict, v12_room_ids(room_version_id), enforce);

    if verdict != StrippedCreateVerdict::Valid {
        debug_warn!(?verdict, %room_id, drop_create, "MSC4311 knock create-event validation failed");
    }

    let events = knock_room_state.iter().filter_map(|event| match event {
        RawStrippedState::Pdu(raw) => serde_json::from_str::<CanonicalJsonObject>(raw.get()).ok(),
        RawStrippedState::Stripped(raw) => {
            serde_json::from_str::<CanonicalJsonObject>(raw.json().get()).ok()
        }
        _ => None,
    });

    let mut state_map = HashMap::new();

    for event in events {
        let Some(state_key) = event.get("state_key").and_then(CanonicalJsonValue::as_str) else {
            debug_warn!("send_knock stripped state event missing state_key: {event:?}");
            continue;
        };

        let Some(event_type) = event.get("type").and_then(CanonicalJsonValue::as_str) else {
            debug_warn!("send_knock stripped state event missing event type: {event:?}");
            continue;
        };

        let event_type = StateEventType::from(event_type);

        if drop_create && event_type == StateEventType::RoomCreate && state_key.is_empty() {
            debug_warn!(%room_id, "dropping unvalidated create event from knock state");
            continue;
        }

        let event_id = gen_event_id(&event, room_version_id)?;

        let shortstatekey = self
            .services
            .short
            .get_or_create_shortstatekey(&event_type, state_key)
            .await;

        self.services.outlier.add_pdu_outlier(&event_id, &event);

        state_map.insert(shortstatekey, event_id);
    }

    Ok(state_map)
}

#[implement(Service)]
async fn make_knock_request(
    &self,
    sender_user: &UserId,
    room_id: &RoomId,
    servers: &[OwnedServerName],
) -> Result<(prepare_knock_event::v1::Response, OwnedServerName)> {
    let mut attempts: usize = 0;
    let mut last_error = Err!(BadServerResponse(
        "No server available to assist in knocking."
    ));

    for remote_server in servers {
        if self.services.server_state.server_is_ours(remote_server) {
            continue;
        }

        info!("Asking {remote_server} for make_knock ({attempts})");

        let mut request =
            prepare_knock_event::v1::Request::new(room_id.to_owned(), sender_user.to_owned());
        request.ver = supported_room_versions();

        let response = self
            .services
            .federation
            .execute(remote_server, request)
            .await;

        trace!("make_knock response: {response:?}");
        attempts = attempts.saturating_add(1);

        match response {
            Ok(response) => return Ok((response, remote_server.clone())),
            Err(e) => last_error = Err(e),
        }

        if attempts > MAKE_KNOCK_ATTEMPTS {
            warn!(
                "{MAKE_KNOCK_ATTEMPTS} servers failed to provide valid make_knock response, \
                 assuming no server can assist in knocking."
            );

            return Err!(BadServerResponse(
                "No server available to assist in knocking."
            ));
        }
    }

    last_error
}
