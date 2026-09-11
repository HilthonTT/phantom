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
