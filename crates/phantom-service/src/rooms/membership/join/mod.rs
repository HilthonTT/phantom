mod event;
mod ingest;
mod local;
mod remote;

use std::collections::HashMap;

use futures::FutureExt;
use phantom_core::{Err, Result, debug_warn, implement};
use ruma::{
    CanonicalJsonObject, OwnedEventId, OwnedServerName, RoomId, RoomOrAliasId, UserId,
    events::room::member::MembershipState,
};

use super::Service;
use crate::rooms::state::RoomMutexGuard;

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

/// Room state as `(shortstatekey, event_id)`, the form both join paths resolve
/// to before it is compressed and forced.
pub(super) type StateIds = HashMap<u64, OwnedEventId>;

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
