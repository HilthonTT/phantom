//! The federation half of third-party invites: the identity server's callback
//! when an invited address is bound, and the exchange that turns a pending
//! invite into a real one on the inviter's server.

use axum::extract::State;
use phantom_core::{Err, Result, err, info, warn};
use phantom_service::Services;
use ruma::{
    api::federation::thirdparty::{bind_callback, exchange_invite},
    events::{
        StateEventType,
        room::member::{MembershipState, RoomMemberEventContent, ThirdPartyInvite},
    },
};

use super::reject_forbidden_room_server;
use crate::{router::Ruma, server::access::require_known_room};

/// # `PUT /_matrix/federation/v1/3pid/onbind`
///
/// An identity server telling us one of our users bound an address that has
/// pending invites. Each is exchanged with the inviter's server, or here when
/// the inviter is ours.
///
/// The request is unauthenticated, as the spec has it: what makes an invite
/// genuine is the identity server's signature inside it, which the room's
/// auth rules check when the invite is appended. An invite that fails is
/// logged and the rest still go, since the identity server cannot act on a
/// partial failure anyway.
pub(crate) async fn bind_callback_route(
    State(services): State<crate::router::State>,
    body: Ruma<bind_callback::v1::Request>,
) -> Result<bind_callback::v1::Response> {
    if !services.server_state.user_is_local(&body.mxid) {
        return Err!(Request(InvalidParam(
            "{} is not a user of this server.",
            body.mxid
        )));
    }

    for invite in &body.invites {
        if invite.mxid != body.mxid {
            warn!(mxid = %invite.mxid, "Skipping a bound invite for another user");
            continue;
        }

        match exchange(&services, invite).await {
            Ok(()) => info!(
                room_id = %invite.room_id,
                sender = %invite.sender,
                mxid = %invite.mxid,
                "Exchanged a third-party invite"
            ),
            Err(e) => warn!(
                room_id = %invite.room_id,
                sender = %invite.sender,
                "Could not exchange a third-party invite: {e}"
            ),
        }
    }

    Ok(bind_callback::v1::Response::new())
}

async fn exchange(services: &Services, invite: &bind_callback::v1::ThirdPartyInvite) -> Result {
    let membership = &services.rooms.membership;

    if services.server_state.user_is_local(&invite.sender) {
        return membership
            .exchange_third_party_invite(
                &invite.room_id,
                &invite.sender,
                &invite.mxid,
                invite.signed.clone(),
            )
            .await;
    }

    // The inviter's server fills the display name in from the room's pending
    // invite; it is required here, so send ours when this server is in the
    // room too, and the bound account otherwise.
    let token = invite
        .signed
        .deserialize()
        .map(|signed| signed.token)
        .map_err(|e| err!(Request(BadJson("The signed invite is unreadable: {e}"))))?;

    let display_name = membership
        .third_party_invite_display_name(&invite.room_id, &token)
        .await
        .unwrap_or_else(|_| invite.mxid.to_string());

    let request = exchange_invite::v1::Request::with_third_party_invite(
        invite.room_id.clone(),
        invite.sender.clone(),
        invite.mxid.clone(),
        ThirdPartyInvite::new(display_name, invite.signed.clone()),
    )?;

    services
        .federation
        .execute(invite.sender.server_name(), request)
        .await?;

    Ok(())
}

/// # `PUT /_matrix/federation/v1/exchange_third_party_invite/{roomId}`
///
/// A bound user's server asking us, as the inviter's, to issue the invite a
/// pending third-party invite promised. The invited user must belong to the
/// server asking.
pub(crate) async fn exchange_third_party_invite_route(
    State(services): State<crate::router::State>,
    body: Ruma<exchange_invite::v1::Request>,
) -> Result<exchange_invite::v1::Response> {
    let origin = body.origin();

    reject_forbidden_room_server(&services, origin, &body.room_id)?;
    require_known_room(&services, &body.room_id, origin).await?;

    if body.kind != StateEventType::RoomMember {
        return Err!(Request(InvalidParam(
            "Only m.room.member can be exchanged."
        )));
    }

    if body.state_key.server_name() != origin {
        return Err!(Request(Forbidden(
            "{origin} cannot exchange an invite for {}.",
            body.state_key
        )));
    }

    let content: RoomMemberEventContent = body
        .content
        .deserialize()
        .map_err(|e| err!(Request(BadJson("The invite's content is unreadable: {e}"))))?;

    if content.membership != MembershipState::Invite {
        return Err!(Request(InvalidParam("Only an invite can be exchanged.")));
    }

    let Some(third_party_invite) = content.third_party_invite else {
        return Err!(Request(InvalidParam(
            "The invite carries no third_party_invite."
        )));
    };

    services
        .rooms
        .membership
        .exchange_third_party_invite(
            &body.room_id,
            &body.sender,
            &body.state_key,
            third_party_invite.signed,
        )
        .await?;

    Ok(exchange_invite::v1::Response::new())
}
