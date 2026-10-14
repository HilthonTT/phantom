use axum::extract::State;
use futures::{FutureExt, join};
use phantom_core::{Err, Result, debug_info};
use phantom_service::{Services, net::identity::InviteContext};
use ruma::{
    RoomId, UserId,
    api::client::membership::{
        Invite3pid,
        invite_user::{
            self,
            v3::{InvitationRecipient, InviteUserId},
        },
    },
    events::room::{
        member::MembershipState,
        third_party_invite::{PublicKey, RoomThirdPartyInviteEventContent},
    },
};

use super::banned_room_check;
use crate::{
    client::utils::invite_check,
    router::{ClientIp, Ruma},
};

/// # `POST /_matrix/client/r0/rooms/{roomId}/invite`
///
/// Tries to send an invite event into the room.
#[tracing::instrument(skip_all, fields(%client), name = "invite")]
pub(crate) async fn invite_user_route(
    State(services): State<crate::router::State>,
    ClientIp(client): ClientIp,
    body: Ruma<invite_user::v3::Request>,
) -> Result<invite_user::v3::Response> {
    let sender_user = body.sender_user();

    let room_id = &body.room_id;

    invite_check(&services, sender_user, room_id).await?;

    banned_room_check(&services, sender_user, room_id, None, client).await?;

    match &body.recipient {
        InvitationRecipient::UserId(InviteUserId {
            user_id, reason, ..
        }) => invite_user(&services, sender_user, user_id, room_id, reason.as_ref()).await?,
        InvitationRecipient::ThirdPartyId(invite) => {
            invite_third_party(&services, sender_user, room_id, invite).await?;
        }
        _ => return Err!(Request(InvalidParam("Unknown kind of invite recipient."))),
    }

    Ok(invite_user::v3::Response::new())
}

async fn invite_user(
    services: &Services,
    sender_user: &UserId,
    user_id: &UserId,
    room_id: &RoomId,
    reason: Option<&String>,
) -> Result {
    // TODO: this should be in the service, but moving it from here would run the
    // sender's ignore-list check before the banned check, revealing the ignore
    // state to the sending user if the recipient is banned
    let member = services.rooms.state_accessor.get_member(room_id, user_id);

    let ignored = services.users.user_is_ignored(user_id, sender_user);

    let (member, ignored) = join!(member, ignored);

    if member.is_ok_and(|member| member.membership == MembershipState::Ban) {
        return Err!(Request(Forbidden("User is banned from this room.")));
    }

    if ignored {
        // silently drop the invite to the recipient if they've been ignored by the
        // sender, pretend it worked
        return Ok(());
    }

    services
        .rooms
        .membership
        .invite(sender_user, user_id, room_id, reason, false)
        .boxed()
        .await
}

/// Invites whoever holds an email address or phone number: the account it
/// is bound to when the identity server knows one, and otherwise a pending
/// invite the identity server tells the address about, completed once its
/// owner binds it.
async fn invite_third_party(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
    invite: &Invite3pid,
) -> Result {
    let Invite3pid {
        id_server,
        id_access_token,
        medium,
        address,
        ..
    } = invite;

    let identity = &services.identity;

    if let Some(user_id) = identity
        .lookup(id_server, id_access_token, medium, address)
        .await?
    {
        debug_info!(%user_id, "Third-party identifier is bound; inviting the account");

        return invite_user(services, sender_user, &user_id, room_id, None).await;
    }

    let context = invite_context(services, sender_user, room_id).await;
    let stored = identity
        .store_invite(
            id_server,
            id_access_token,
            medium,
            address,
            room_id,
            sender_user,
            &context,
        )
        .await?;

    // `public_key` and `key_validity_url` are the older single-key fields,
    // still required; they repeat the first of `public_keys`.
    let first = &stored.public_keys[0];
    let mut content = RoomThirdPartyInviteEventContent::new(
        stored.display_name,
        first.key_validity_url.clone(),
        first.public_key.clone(),
    );
    content.public_keys = Some(
        stored
            .public_keys
            .into_iter()
            .map(|key| {
                let mut public_key = PublicKey::new(key.public_key);
                public_key.key_validity_url = Some(key.key_validity_url);
                public_key
            })
            .collect(),
    );

    services
        .rooms
        .membership
        .send_third_party_invite(sender_user, room_id, &stored.token, &content)
        .await
}

/// What the identity server is told of the room and the sender, so the
/// message it sends the address can say who invited them where.
async fn invite_context(
    services: &Services,
    sender_user: &UserId,
    room_id: &RoomId,
) -> InviteContext {
    let state = &services.rooms.state_accessor;

    InviteContext {
        room_alias: state
            .get_canonical_alias(room_id)
            .await
            .ok()
            .map(|alias| alias.to_string()),
        room_avatar_url: state
            .get_avatar(room_id)
            .await
            .into_option()
            .and_then(|avatar| avatar.url)
            .map(|url| url.to_string()),
        room_join_rules: Some(state.get_join_rules(room_id).await.as_str().to_owned()),
        room_name: state.get_name(room_id).await.ok(),
        room_type: state
            .get_room_type(room_id)
            .await
            .ok()
            .map(|room_type| room_type.to_string()),
        sender_display_name: services.profile.displayname(sender_user).await.ok(),
        sender_avatar_url: services
            .profile
            .avatar_url(sender_user)
            .await
            .ok()
            .map(|url| url.to_string()),
    }
}
