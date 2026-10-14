//! Invites by email address or phone number, which take two steps.
//!
//! Inviting an identifier no one has bound yet puts an
//! `m.room.third_party_invite` in the room, keyed by a token an identity
//! server issued along with the keys it signs with. When the identifier's
//! owner binds it to a Matrix account, the identity server tells that
//! account's server, which asks the inviter's server to exchange the pending
//! invite for a real one: an `m.room.member` invite carrying the identity
//! server's signature over the account and the token. The room's auth rules
//! are what check that signature against the keys the first event named.

use futures::FutureExt;
use phantom_core::{Err, Result, err, implement, matrix::PduBuilder};
use ruma::{
    RoomId, UserId,
    events::{
        StateEventType, TimelineEventType,
        room::{
            member::{SignedContent, ThirdPartyInvite},
            third_party_invite::RoomThirdPartyInviteEventContent,
        },
    },
    serde::Raw,
};

use serde_json::{json, value::to_raw_value};

use super::Service;

/// Puts a pending third-party invite in the room, from `sender`, under the
/// `token` the identity server issued for it.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, content), fields(%sender, %room_id))]
pub async fn send_third_party_invite(
    &self,
    sender: &UserId,
    room_id: &RoomId,
    token: &str,
    content: &RoomThirdPartyInviteEventContent,
) -> Result {
    if !self.services.state_cache.is_joined(sender, room_id).await {
        return Err!(Request(Forbidden(
            "You must be joined in the room you are trying to invite from."
        )));
    }

    let state_lock = self.services.state.mutex.lock(room_id).await;

    self.services
        .timeline
        .build_and_append_pdu(
            PduBuilder::state(token.to_owned(), content),
            sender,
            room_id,
            &state_lock,
        )
        .await?;

    Ok(())
}

/// Turns a pending third-party invite into a real one, now that its
/// identifier is bound to `user_id`. `sender` sent the pending invite and is
/// local: only its own server can sign an event in its name.
///
/// Nothing here checks the identity server's signature; the auth rules do,
/// when the invite is appended, against the keys the pending invite named.
#[implement(Service)]
#[tracing::instrument(level = "debug", skip(self, signed), fields(%sender, %user_id, %room_id))]
pub async fn exchange_third_party_invite(
    &self,
    room_id: &RoomId,
    sender: &UserId,
    user_id: &UserId,
    signed: Raw<SignedContent>,
) -> Result {
    if !self.services.server_state.user_is_local(sender) {
        return Err!(Request(Forbidden(
            "{sender} is not a user of this server, so it cannot invite in their name."
        )));
    }

    let content = signed
        .deserialize()
        .map_err(|e| err!(Request(BadJson("The signed invite is unreadable: {e}"))))?;

    if content.mxid != user_id {
        return Err!(Request(InvalidParam(
            "The signed invite is for {}, not {user_id}.",
            content.mxid
        )));
    }

    let pending: RoomThirdPartyInviteEventContent = self
        .services
        .state_accessor
        .room_state_get_content(
            room_id,
            &StateEventType::RoomThirdPartyInvite,
            &content.token,
        )
        .await
        .map_err(|_| {
            err!(Request(NotFound(
                "The room holds no such third-party invite."
            )))
        })?;

    // An identity server tells every server of a bound user, and may retry;
    // an invite already answered is done.
    if self.services.state_cache.is_joined(user_id, room_id).await
        || self.services.state_cache.is_invited(user_id, room_id).await
    {
        return Ok(());
    }

    let mut member = self.invite_content(user_id, None, false).await;
    member.third_party_invite = Some(ThirdPartyInvite::new(pending.display_name, signed));

    self.invite_with_content(sender, user_id, room_id, member)
        .boxed()
        .await?;

    // The signed block is public once the invite is in the room, and the
    // callback that exchanges it is unauthenticated, so a pending invite is
    // used up here: otherwise anyone could post it again after the user left
    // or declined, and have them invited again each time.
    self.revoke_third_party_invite(sender, room_id, &content.token)
        .await
}

/// Revokes the pending invite `token` names, by replacing it with empty
/// content: the exchange then finds nothing to exchange, and the auth rules,
/// which need the pending invite's public keys, refuse a member event that
/// still cites it.
#[implement(Service)]
async fn revoke_third_party_invite(
    &self,
    sender: &UserId,
    room_id: &RoomId,
    token: &str,
) -> Result {
    let state_lock = self.services.state.mutex.lock(room_id).await;

    let revoked = PduBuilder {
        event_type: TimelineEventType::RoomThirdPartyInvite,
        content: to_raw_value(&json!({}))?,
        state_key: Some(token.into()),
        ..PduBuilder::default()
    };

    self.services
        .timeline
        .build_and_append_pdu(revoked, sender, room_id, &state_lock)
        .await?;

    Ok(())
}

/// The display name of the pending invite `token` names in the room, which
/// is what an exchange sends on, since the identity server's callback does
/// not carry it.
#[implement(Service)]
pub async fn third_party_invite_display_name(
    &self,
    room_id: &RoomId,
    token: &str,
) -> Result<String> {
    self.services
        .state_accessor
        .room_state_get_content(room_id, &StateEventType::RoomThirdPartyInvite, token)
        .await
        .map(|pending: RoomThirdPartyInviteEventContent| pending.display_name)
}
