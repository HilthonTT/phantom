//! Tearing an account down.
//!
//! Deactivation is not deletion. The user id is never handed to anyone else,
//! and the events the account sent stay in the rooms they were sent to,
//! because a room's history belongs to the room rather than to the sender.
//! What goes is everything that makes the account usable — the password and
//! the devices that could log in with it — and everything on it that is only
//! ever the user's own: the profile, and the membership of every room.
//!
//! A user may also ask, under MSC4025, that their non-event data be erased
//! rather than merely left behind. That is what `erase` reaches: the account
//! data the user set, global and per-room, including for rooms they had
//! already left. It stops there, at data no one else can see. Nothing in a
//! room's timeline is touched.
//!
//! Leaving the rooms is [`leave`], which both demotes the user out of any
//! power they hold and sends the leave events that tell each room's other
//! members the account is gone.

mod leave;

use std::sync::Arc;

use futures::StreamExt;
use phantom_core::Result;
use ruma::{OwnedRoomId, UserId};

use crate::{Dep, account_data, rooms, users};

pub struct Service {
    services: Services,
}

struct Services {
    account_data: Dep<account_data::Service>,
    state: Dep<rooms::state::Service>,
    state_accessor: Dep<rooms::state_accessor::Service>,
    state_cache: Dep<rooms::state_cache::Service>,
    timeline: Dep<rooms::timeline::Service>,
    users: Dep<users::Service>,
}

impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>>
    where
        Self: Sized,
    {
        Ok(Arc::new(Self {
            services: Services {
                account_data: args.depend::<account_data::Service>("account_data"),
                state: args.depend::<rooms::state::Service>("rooms::state"),
                state_accessor: args
                    .depend::<rooms::state_accessor::Service>("rooms::state_accessor"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                timeline: args.depend::<rooms::timeline::Service>("rooms::timeline"),
                users: args.depend::<users::Service>("users"),
            },
        }))
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Runs through all the deactivation steps:
    ///
    /// - Mark as deactivated
    /// - Removing display name
    /// - Removing avatar URL and blurhash
    /// - Removing all profile data
    /// - Leaving all rooms (and forgets all of them)
    ///
    /// When `erase` is `true`, additionally erase non-event data per
    /// MSC4025: all global and per-room account data for the user.
    #[tracing::instrument(skip(self), level = "debug")]
    pub async fn full_deactivate(&self, user_id: &UserId, erase: bool) -> Result {
        self.services.users.deactivate_account(user_id).await?;
        self.services.users.clear_profile(user_id).await;

        self.demote_self(user_id).await?;

        let all_rooms = self.all_rooms(user_id).await;

        if erase {
            self.erase_account_data(user_id, &all_rooms).await;
        }

        for room_id in &all_rooms {
            self.leave_room(user_id, room_id).await;
            self.services.state_cache.forget(room_id, user_id);
        }

        Ok(())
    }

    /// Every room the deactivating user still has a foot in.
    ///
    /// Joined, invited and knocked: each is a membership that would keep
    /// showing the room in the user's sync, so each has to be left. Rooms
    /// already left are not here — there is nothing left to leave — which is
    /// why [`erase_account_data`](Self::erase_account_data) gathers its own
    /// set rather than reusing this one.
    async fn all_rooms(&self, user_id: &UserId) -> Vec<OwnedRoomId> {
        let joined = self
            .services
            .state_cache
            .rooms_joined(user_id)
            .map(ToOwned::to_owned);

        let invited = self
            .services
            .state_cache
            .rooms_invited(user_id)
            .map(|(room_id, _)| room_id);

        let knocked = self
            .services
            .state_cache
            .rooms_knocked(user_id)
            .map(|(room_id, _)| room_id);

        joined.chain(invited).chain(knocked).collect().await
    }

    /// Erases the user's account data, global and in every room they were
    /// ever in.
    ///
    /// Rooms already left are included: the user's tags and read markers for
    /// a room outlive their membership of it, so erasing only the rooms they
    /// are still in would leave the older ones behind.
    async fn erase_account_data(&self, user_id: &UserId, all_rooms: &[OwnedRoomId]) {
        self.services.account_data.erase_user(user_id, None).await;

        let rooms_left: Vec<OwnedRoomId> = self
            .services
            .state_cache
            .rooms_left(user_id)
            .map(|(room_id, _)| room_id)
            .collect()
            .await;

        for room_id in all_rooms.iter().chain(rooms_left.iter()) {
            self.services
                .account_data
                .erase_user(user_id, Some(room_id))
                .await;
        }
    }
}
