use futures::{FutureExt, StreamExt, stream};
use phantom_core::{
    Err, Result, err, implement,
    matrix::{PduBuilder, PduEvent, pdu::gen_event_id_canonical_json},
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedServerName, RoomId, RoomVersionId, UserId,
    api::{
        error::ErrorKind,
        federation::membership::{RawStrippedState, create_invite},
    },
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent},
    },
};

use super::{Service, outgoing_pdu};

#[implement(Service)]
#[tracing::instrument(level = "debug", skip_all, fields(%sender_user, %room_id, %user_id))]
pub async fn invite(
    &self,
    sender_user: &UserId,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<&String>,
    is_direct: bool,
) -> Result {
    if self.services.server_state.user_is_local(user_id) {
        self.local_invite(sender_user, user_id, room_id, reason, is_direct)
            .boxed()
            .await
    } else {
        self.remote_invite(sender_user, user_id, room_id, reason, is_direct)
            .boxed()
            .await
    }
}

#[implement(Service)]
async fn invite_content(
    &self,
    user_id: &UserId,
    reason: Option<&String>,
    is_direct: bool,
) -> RoomMemberEventContent {
    let mut content = RoomMemberEventContent::new(MembershipState::Invite);
    content.is_direct = Some(is_direct);
    content.reason = reason.cloned();

    self.services
        .profile
        .fill_profile_data(user_id, &mut content)
        .await;

    content
}

#[implement(Service)]
#[tracing::instrument(name = "remote", level = "debug", skip_all)]
async fn remote_invite(
    &self,
    sender_user: &UserId,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<&String>,
    is_direct: bool,
) -> Result {
    let (pdu, pdu_json, invite_room_state, room_version_id) = {
        let state_lock = self.services.state.mutex.lock(room_id).await;

        let content = self.invite_content(user_id, reason, is_direct).await;

        let (pdu, pdu_json) = self
            .services
            .timeline
            .create_hash_and_sign_event(
                PduBuilder::state(user_id.to_string(), &content),
                sender_user,
                room_id,
                &state_lock,
            )
            .await?;

        let room_version_id = self.services.state.get_room_version(room_id).await?;

        let invite_room_state = self.summary_pdus(&pdu, &pdu_json, &room_version_id).await;

        drop(state_lock);

        (pdu, pdu_json, invite_room_state, room_version_id)
    };

    let request = create_invite::v2::Request::new(
        room_id.to_owned(),
        pdu.event_id.clone(),
        room_version_id.clone(),
        outgoing_pdu(pdu_json, &room_version_id),
        invite_room_state,
    );

    let response = self
        .services
        .federation
        .execute(user_id.server_name(), request)
        .await
        .map_err(|e| match e.kind() {
            ErrorKind::IncompatibleRoomVersion(_) | ErrorKind::UnsupportedRoomVersion => {
                err!(Request(UnsupportedRoomVersion(
                    "Server {} does not support room version {room_version_id}.",
                    user_id.server_name(),
                )))
            }
            ErrorKind::MissingParam => err!(BadServerResponse(
                "Remote server could not validate the invite's create event."
            )),
            _ => e,
        })?;

    let (event_id, value) = gen_event_id_canonical_json(&response.event, &room_version_id)
        .map_err(|e| {
            err!(Request(BadJson(warn!(
                "Could not convert event to canonical JSON: {e}"
            ))))
        })?;

    if pdu.event_id != event_id {
        return Err!(Request(BadJson(warn!(
            "Server {} sent event {event_id} when {} was expected",
            user_id.server_name(),
            pdu.event_id
        ))));
    }

    let origin = value
        .get("origin")
        .and_then(CanonicalJsonValue::as_str)
        .and_then(|origin| OwnedServerName::try_from(origin).ok())
        .unwrap_or_else(|| user_id.server_name().to_owned());

    let pdu_id = self
        .services
        .event_handler
        .handle_incoming_pdu(&origin, room_id, &event_id, value, true)
        .await?
        .ok_or_else(|| {
            err!(Request(InvalidParam(
                "Could not accept incoming PDU as timeline event."
            )))
        })?;

    self.services.sending.send_pdu_room(room_id, &pdu_id).await
}

#[implement(Service)]
#[tracing::instrument(name = "local", level = "debug", skip_all)]
async fn local_invite(
    &self,
    sender_user: &UserId,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<&String>,
    is_direct: bool,
) -> Result {
    if !self
        .services
        .state_cache
        .is_joined(sender_user, room_id)
        .await
    {
        return Err!(Request(Forbidden(
            "You must be joined in the room you are trying to invite from."
        )));
    }

    let state_lock = self.services.state.mutex.lock(room_id).await;

    let content = self.invite_content(user_id, reason, is_direct).await;

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(user_id.to_string(), &content),
            sender_user,
            room_id,
            &state_lock,
        )
        .await?;

    drop(state_lock);

    Ok(())
}

#[implement(Service)]
async fn summary_pdus(
    &self,
    event: &PduEvent,
    event_json: &CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Vec<RawStrippedState> {
    let cells = [
        (StateEventType::RoomCreate, ""),
        (StateEventType::RoomJoinRules, ""),
        (StateEventType::RoomCanonicalAlias, ""),
        (StateEventType::RoomName, ""),
        (StateEventType::RoomAvatar, ""),
        (StateEventType::RoomMember, event.sender.as_str()),
        (StateEventType::RoomEncryption, ""),
        (StateEventType::RoomTopic, ""),
    ];

    let membership = outgoing_pdu(event_json.clone(), room_version);

    stream::iter(cells)
        .filter_map(async |(event_type, state_key)| {
            let pdu = self
                .services
                .state_accessor
                .room_state_get(&event.room_id, &event_type, state_key)
                .await
                .ok()?;

            let pdu_json = self
                .services
                .timeline
                .get_pdu_json(&pdu.event_id)
                .await
                .ok()?;

            Some(RawStrippedState::Pdu(outgoing_pdu(pdu_json, room_version)))
        })
        .chain(stream::once(async { RawStrippedState::Pdu(membership) }))
        .collect()
        .await
}
