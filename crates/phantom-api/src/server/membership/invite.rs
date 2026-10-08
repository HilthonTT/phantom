use std::mem::take;

use axum::extract::State;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::StreamExt;
use phantom_core::{
    Err, Error, Result, debug_warn, err,
    hash::sha256,
    json::to_canonical_object,
    matrix::pdu::{PduEvent, gen_event_id},
};
use phantom_service::{
    Services,
    rooms::membership::{
        StrippedCreateVerdict, dedup_stripped_state, enforce_stripped_create, into_client_stripped,
        v12_room_ids, without_member,
    },
};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedRoomId, OwnedTransactionId, OwnedUserId,
    RoomVersionId, ServerName, UInt, UserId,
    api::{
        appservice::event::push_events,
        error::{ErrorKind, IncompatibleRoomVersionErrorData},
        federation::membership::{RawStrippedState, create_invite},
    },
    events::{
        AnyStrippedStateEvent, GlobalAccountDataEventType, StateEventType,
        push_rules::PushRulesEvent,
        room::member::{MembershipState, RoomMemberEventContent},
    },
    push::Ruleset,
    serde::{JsonObject, Raw},
};
use serde::Deserialize;

use super::reject_forbidden_room_server;
use crate::router::{ClientIp, Ruma};

#[derive(Deserialize)]
struct DirectFlag {
    #[serde(default)]
    is_direct: bool,
}

#[tracing::instrument(skip_all, fields(%client), name = "invite")]
pub(crate) async fn create_invite_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    mut body: Ruma<create_invite::v2::Request>,
) -> Result<create_invite::v2::Response> {
    services.sending.notify_peer_alive(body.origin()).await;

    validate_request(&services, &body).await?;

    let stripped_state = dedup_stripped_state(take(&mut body.body.invite_room_state));
    enforce_stripped_state(&services, &body, &stripped_state).await?;

    let (mut signed_event, invited_user) = parse_invite_event(&services, &body).await?;
    sign_event(&services, &mut signed_event, &body.room_version)?;

    let sender = invite_sender(&signed_event, body.origin())?;
    check_invite_permitted(&services, &body, &invited_user).await?;

    let pdu = build_pdu(&body)?;

    let invite_state: Vec<_> = without_member(stripped_state, &invited_user)
        .filter_map(|state| into_client_stripped(&body.room_id, state))
        .chain([pdu.to_stripped_state_event()])
        .collect();

    let federation_lock = services
        .rooms
        .event_handler
        .lock_federation(&body.room_id)
        .await;

    record_invite(&services, &body, &invited_user, &sender, invite_state, &pdu).await?;

    drop(federation_lock);

    let event = services
        .federation
        .format_pdu(signed_event, Some(&body.room_version))
        .await;

    Ok(create_invite::v2::Response::new(event))
}

async fn validate_request(services: &Services, body: &Ruma<create_invite::v2::Request>) -> Result {
    services
        .rooms
        .event_handler
        .acl_check(body.origin(), &body.room_id)
        .await?;

    if !services
        .rooms
        .membership
        .supported_room_version(&body.room_version)
    {
        return Err(Error::BadRequest(
            ErrorKind::IncompatibleRoomVersion(IncompatibleRoomVersionErrorData::new(
                body.room_version.clone(),
            )),
            "Server does not support this room version.",
        ));
    }

    reject_forbidden_room_server(services, body.origin(), &body.room_id)
}

async fn enforce_stripped_state(
    services: &Services,
    body: &Ruma<create_invite::v2::Request>,
    stripped_state: &[RawStrippedState],
) -> Result {
    let verdict = services
        .rooms
        .membership
        .validate_stripped_create(stripped_state, &body.room_id, &body.room_version)
        .await?;

    if verdict != StrippedCreateVerdict::Valid {
        debug_warn!(
            "MSC4311 invite create event validation failed for {}: {verdict:?}",
            body.room_id
        );
    }

    let enforce = services
        .server
        .config
        .membership
        .enforce_stripped_state_pdu_validation;

    if enforce_stripped_create(verdict, v12_room_ids(&body.room_version), enforce) {
        return Err!(Request(MissingParam(
            "The invite's m.room.create event is missing or does not validate for this room."
        )));
    }

    Ok(())
}

async fn parse_invite_event(
    services: &Services,
    body: &Ruma<create_invite::v2::Request>,
) -> Result<(CanonicalJsonObject, OwnedUserId)> {
    let signed_event = to_canonical_object(&body.event)
        .map_err(|_| err!(Request(InvalidParam("Invite event is invalid."))))?;

    let room_id: OwnedRoomId = string_field(&signed_event, "room_id")?
        .try_into()
        .map_err(|e| err!(Request(InvalidParam("Invalid room_id property: {e}"))))?;

    if body.room_id != room_id {
        return Err!(Request(InvalidParam(
            "Event room_id does not match the request path."
        )));
    }

    let kind = StateEventType::from(string_field(&signed_event, "type")?);
    if kind != StateEventType::RoomMember {
        return Err!(Request(InvalidParam("Event must be m.room.member type.")));
    }

    let invited_user: OwnedUserId = string_field(&signed_event, "state_key")?
        .try_into()
        .map_err(|e| err!(Request(InvalidParam("Invalid state_key property: {e}"))))?;

    if !services.server_state.user_is_local(&invited_user) {
        return Err!(Request(InvalidParam(
            "User does not belong to this homeserver."
        )));
    }

    let content: RoomMemberEventContent = signed_event
        .get("content")
        .cloned()
        .map(|content| serde_json::from_value(content.into()))
        .transpose()
        .map_err(|e| {
            err!(Request(InvalidParam(
                "Invalid content object in event: {e}"
            )))
        })?
        .ok_or_else(|| err!(Request(BadJson("Missing content in event."))))?;

    if content.membership != MembershipState::Invite {
        return Err!(Request(InvalidParam("Event membership must be invite.")));
    }

    services
        .rooms
        .event_handler
        .acl_check(invited_user.server_name(), &body.room_id)
        .await?;

    Ok((signed_event, invited_user))
}

fn sign_event(
    services: &Services,
    signed_event: &mut CanonicalJsonObject,
    room_version: &RoomVersionId,
) -> Result {
    services
        .server_keys
        .hash_and_sign_event(signed_event, room_version)
        .map_err(|e| err!(Request(InvalidParam("Failed to sign event: {e}"))))?;

    let event_id = gen_event_id(signed_event, room_version)?;
    signed_event.insert(
        "event_id".into(),
        CanonicalJsonValue::String(event_id.into()),
    );

    Ok(())
}

fn invite_sender(signed_event: &CanonicalJsonObject, origin: &ServerName) -> Result<OwnedUserId> {
    let sender: OwnedUserId = string_field(signed_event, "sender")?
        .try_into()
        .map_err(|e| err!(Request(InvalidParam("Invalid sender property: {e}"))))?;

    if sender.server_name() != origin {
        return Err!(Request(Forbidden(
            "Can only send invites on behalf of your users."
        )));
    }

    let event_origin = signed_event
        .get("origin")
        .and_then(CanonicalJsonValue::as_str);

    if event_origin.is_some_and(|event_origin| event_origin != origin.as_str()) {
        return Err!(Request(Forbidden("Can only send events from your origin.")));
    }

    Ok(sender)
}

async fn check_invite_permitted(
    services: &Services,
    body: &Ruma<create_invite::v2::Request>,
    invited_user: &UserId,
) -> Result {
    let block_non_admin = services.server.config.membership.block_non_admin_invites;
    let room_banned = services.rooms.metadata.is_banned(&body.room_id).await;

    if !room_banned && !block_non_admin {
        return Ok(());
    }

    if services.users.is_admin(invited_user).await {
        return Ok(());
    }

    if room_banned {
        return Err!(Request(Forbidden(
            "This room is banned on this homeserver."
        )));
    }

    Err!(Request(Forbidden(
        "This server does not allow room invites."
    )))
}

fn build_pdu(body: &Ruma<create_invite::v2::Request>) -> Result<PduEvent> {
    let mut event: JsonObject = serde_json::from_str(body.event.get())
        .map_err(|e| err!(Request(BadJson("Invalid invite event PDU: {e}"))))?;

    event.insert("event_id".into(), "$placeholder".into());

    serde_json::from_value(event.into())
        .map_err(|e| err!(Request(BadJson("Invalid invite event PDU: {e}"))))
}

async fn record_invite(
    services: &Services,
    body: &Ruma<create_invite::v2::Request>,
    invited_user: &UserId,
    sender: &UserId,
    invite_state: Vec<Raw<AnyStrippedStateEvent>>,
    pdu: &PduEvent,
) -> Result {
    let state_lock = services.rooms.state.mutex.lock(&*body.room_id).await;

    if services
        .rooms
        .state_cache
        .server_in_room(services.server_state.server_name(), &body.room_id)
        .await
    {
        return Ok(());
    }

    if services
        .rooms
        .state_accessor
        .get_member(&body.room_id, invited_user)
        .await
        .is_ok_and(|member| member.membership == MembershipState::Ban)
    {
        debug_warn!(
            "Recording invite for {invited_user} in {} while local room state shows them banned.",
            body.room_id
        );
    }

    services
        .rooms
        .state_cache
        .update_membership(
            &body.room_id,
            invited_user,
            RoomMemberEventContent::new(MembershipState::Invite),
            sender,
            Some(invite_state),
            None,
            true,
        )
        .await?;

    drop(state_lock);

    let is_direct = pdu
        .get_content()
        .is_ok_and(|content: DirectFlag| content.is_direct);

    services
        .rooms
        .membership
        .auto_accept(&body.room_id, invited_user, sender, is_direct);

    notify_pushers(services, invited_user, pdu).await;
    notify_appservices(services, invited_user, pdu).await
}

async fn notify_pushers(services: &Services, invited_user: &UserId, pdu: &PduEvent) {
    let ruleset = services
        .account_data
        .get_global(invited_user, GlobalAccountDataEventType::PushRules)
        .await
        .map_or_else(
            |_| Ruleset::server_default(invited_user),
            |event: PushRulesEvent| event.content.global,
        );

    services
        .pusher
        .get_pushkeys(invited_user)
        .map(ToOwned::to_owned)
        .for_each(async |pushkey| {
            let Ok(pusher) = services.pusher.get_pusher(invited_user, &pushkey).await else {
                return;
            };

            services
                .pusher
                .send_push_notice(
                    invited_user,
                    UInt::from(1_u32),
                    &pusher,
                    ruleset.clone(),
                    pdu,
                )
                .await
                .ok();
        })
        .await;
}

async fn notify_appservices(services: &Services, invited_user: &UserId, pdu: &PduEvent) -> Result {
    let registrations: Vec<_> = services
        .appservice
        .read()
        .await
        .values()
        .filter(|info| info.is_user_match(invited_user))
        .map(|info| info.registration.clone())
        .collect();

    let txn_id =
        OwnedTransactionId::from(URL_SAFE_NO_PAD.encode(sha256::hash(pdu.event_id.as_bytes())));

    for registration in registrations {
        let request =
            push_events::v1::Request::new(txn_id.clone(), vec![pdu.clone().into_any_event()]);

        services
            .appservice
            .send_request(registration, request)
            .await
            .map_err(|_| {
                err!(BadServerResponse(
                    "Failed to notify appservice about incoming invite."
                ))
            })?;
    }

    Ok(())
}

fn string_field<'a>(event: &'a CanonicalJsonObject, key: &str) -> Result<&'a str> {
    event
        .get(key)
        .and_then(CanonicalJsonValue::as_str)
        .ok_or_else(|| err!(Request(BadJson("Missing {key} in event."))))
}
