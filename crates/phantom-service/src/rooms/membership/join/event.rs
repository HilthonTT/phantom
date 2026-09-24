//! Building the join member event, and the `make_join` server search.

use phantom_core::{Err, Result, err, implement, info, trace, warn};
use ruma::{
    CanonicalJsonObject, CanonicalJsonValue, OwnedEventId, OwnedServerName, OwnedUserId, RoomId,
    RoomVersionId, UserId,
    api::{error::ErrorKind, federation::membership::prepare_join_event},
    events::room::member::{MembershipState, RoomMemberEventContent},
};
use serde_json::value::RawValue as RawJsonValue;

use crate::rooms::membership::{Service, merge_member_content, supported_room_versions};

const INCOMPATIBLE_ROOM_VERSION_LIMIT: usize = 15;

#[implement(Service)]
#[tracing::instrument(name = "make_join", level = "debug", skip_all)]
pub(super) async fn create_join_event(
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
        .profile
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
pub(super) async fn make_join_request(
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
