use std::time::Duration;

use futures::StreamExt;
use phantom_core::{debug, debug_warn, implement, result::LogErr, stream::automatic_width};
use ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId};
use tokio::time::sleep;

use super::{Join, Service};

pub(super) struct Pending {
    room_id: OwnedRoomId,
    user_id: OwnedUserId,
    sender: OwnedUserId,
    is_direct: bool,
}

const ATTEMPTS: u32 = 5;

#[implement(Service)]
pub fn auto_accept(&self, room_id: &RoomId, user_id: &UserId, sender: &UserId, is_direct: bool) {
    let config = &self.services.server.config.membership;
    let server_state = &self.services.server_state;

    let accepts = config.auto_accept_invites
        && (is_direct || !config.auto_accept_invites_direct_only)
        && server_state.user_is_local(user_id)
        && (!config.auto_accept_invites_local_only || server_state.user_is_local(sender));

    if !accepts {
        return;
    }

    self.queue
        .0
        .send(Pending {
            room_id: room_id.to_owned(),
            user_id: user_id.to_owned(),
            sender: sender.to_owned(),
            is_direct,
        })
        .ok();
}

#[implement(Service)]
pub(super) async fn accept_worker(&self) {
    let accepting = self
        .queue
        .1
        .stream()
        .for_each_concurrent(automatic_width(), async |invite| self.accept(invite).await);

    tokio::select! {
        () = accepting => {},
        () = self.services.server.until_shutdown() => {},
    }
}

#[implement(Service)]
#[tracing::instrument(name = "auto_accept", level = "debug", skip_all, fields(%room_id, %user_id))]
async fn accept(
    &self,
    Pending {
        room_id,
        user_id,
        sender,
        is_direct,
    }: Pending,
) {
    if !self.join_invited(&room_id, &user_id, &sender).await {
        return;
    }

    debug!("Accepted the invitation on the user's behalf.");

    if is_direct {
        self.services
            .account_data
            .mark_direct(&user_id, &sender, &room_id)
            .await
            .log_err()
            .ok();
    }
}

#[implement(Service)]
async fn join_invited(&self, room_id: &RoomId, user_id: &UserId, sender: &UserId) -> bool {
    for attempt in 0..ATTEMPTS {
        sleep(retry_delay(attempt)).await;

        if !self.acceptable(room_id, user_id, sender).await {
            return false;
        }

        let joined = self
            .join(Join {
                sender_user: user_id,
                room_id,
                orig_room_id: None,
                reason: None,
                servers: &[],
                is_appservice: false,
                extra_content: None,
            })
            .await;

        match joined {
            Ok(()) => return true,
            Err(e) => debug_warn!(?e, "Automatic invite acceptance attempt failed"),
        }
    }

    false
}

#[implement(Service)]
async fn acceptable(&self, room_id: &RoomId, user_id: &UserId, sender: &UserId) -> bool {
    let (invited, active, ignored) = futures::join!(
        self.services.state_cache.is_invited(user_id, room_id),
        self.services.users.is_active_local(user_id),
        self.services.users.user_is_ignored(sender, user_id),
    );

    invited && active && !ignored
}

fn retry_delay(attempt: u32) -> Duration {
    attempt
        .checked_sub(1)
        .map_or(Duration::ZERO, |retry| Duration::from_secs(1 << retry))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ATTEMPTS, retry_delay};

    #[test]
    fn the_first_attempt_is_immediate_and_retries_double() {
        let delays: Vec<Duration> = (0..ATTEMPTS).map(retry_delay).collect();

        assert_eq!(delays, [0, 1, 2, 4, 8].map(Duration::from_secs));
    }
}
