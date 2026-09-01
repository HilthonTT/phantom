//! Who is online, and for how long they still count as online.
//!
//! Presence is a claim a client makes about a person and then stops
//! refreshing, so nothing ever arrives to say a user went away — the server
//! has to decide that itself. Every update schedules a timer, and when the
//! timer fires the user is moved on: online to unavailable after
//! `presence_idle_timeout_s`, unavailable to offline after
//! `presence_offline_timeout_s`.
//!
//! The timers live in one worker rather than a task each, and the state they
//! carry is a user id and a duration, so a user who keeps pinging simply gets
//! another timer rather than an existing one being found and rescheduled.

mod data;
mod record;

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt, TryFutureExt, stream::FuturesUnordered};
use phantom_core::{
    Result, checked, debug, debug_warn, err, result::LogErr, server::Server, trace,
};
use phantom_database::Database;
use ruma::{OwnedUserId, UInt, UserId, events::presence::PresenceEvent, presence::PresenceState};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    time::sleep,
};

use self::{data::Data, record::Presence};
use crate::{Dep, server_state, users};

pub struct Service {
    timer_sender: mpsc::UnboundedSender<TimerType>,

    /// Held behind a lock because the worker takes it and the trait hands the
    /// worker a shared reference; a second worker would be a bug, and this is
    /// what makes it one that cannot happen.
    timer_receiver: Mutex<mpsc::UnboundedReceiver<TimerType>>,

    /// Signalled by [`Service::interrupt`], which is not async and so cannot
    /// reach the receiver to close it.
    interrupt: Notify,

    timeout_remote_users: bool,
    idle_timeout: u64,
    offline_timeout: u64,
    db: Data,
    services: Services,
}

struct Services {
    server: Arc<Server>,
    db: Arc<Database>,
    server_state: Dep<server_state::Service>,
    users: Dep<users::Service>,
}

type TimerType = (OwnedUserId, Duration);

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let config = &args.server.config;
        let idle_timeout_s = config.presence_idle_timeout_s;
        let offline_timeout_s = config.presence_offline_timeout_s;
        let (timer_sender, timer_receiver) = mpsc::unbounded_channel();

        Ok(Arc::new(Self {
            timer_sender,
            timer_receiver: Mutex::new(timer_receiver),
            interrupt: Notify::new(),
            timeout_remote_users: config.presence_timeout_remote_users,
            idle_timeout: checked!(idle_timeout_s * 1_000)?,
            offline_timeout: checked!(offline_timeout_s * 1_000)?,
            db: Data::new(&args),
            services: Services {
                server: args.server.clone(),
                db: args.db.clone(),
                server_state: args.depend::<server_state::Service>("server_state"),
                users: args.depend::<users::Service>("users"),
            },
        }))
    }

    async fn worker(self: Arc<Self>) -> Result<()> {
        let mut receiver = self.timer_receiver.lock().await;

        let mut presence_timers = FuturesUnordered::new();
        loop {
            tokio::select! {
                () = self.interrupt.notified() => break,
                Some(user_id) = presence_timers.next() => {
                    self.process_presence_timer(&user_id).await.log_err().ok();
                },
                event = receiver.recv() => match event {
                    None => break,
                    Some((user_id, timeout)) => {
                        debug!("Adding timer {}: {user_id} timeout:{timeout:?}", presence_timers.len());
                        presence_timers.push(presence_timer(user_id, timeout));
                    },
                },
            }
        }

        Ok(())
    }

    fn interrupt(&self) {
        self.interrupt.notify_one();
    }

    fn name(&self) -> &str {
        crate::make_name(std::module_path!())
    }
}

impl Service {
    /// Returns the latest presence event for the given user.
    #[inline]
    pub async fn get_presence(&self, user_id: &UserId) -> Result<PresenceEvent> {
        self.db
            .get_presence(user_id)
            .map_ok(|(_, presence)| presence)
            .await
    }

    /// Pings the presence of the given user in the given room, setting the
    /// specified state.
    pub async fn ping_presence(&self, user_id: &UserId, new_state: &PresenceState) -> Result<()> {
        const REFRESH_TIMEOUT: u64 = 60 * 1000;

        let last_presence = self.db.get_presence(user_id).await;
        let state_changed = match last_presence {
            Err(_) => true,
            Ok((_, ref presence)) => presence.content.presence != *new_state,
        };

        let last_last_active_ago = match last_presence {
            Err(_) => 0_u64,
            Ok((_, ref presence)) => presence.content.last_active_ago.unwrap_or_default().into(),
        };

        if !state_changed && last_last_active_ago < REFRESH_TIMEOUT {
            return Ok(());
        }

        let status_msg = match last_presence {
            Ok((_, ref presence)) => presence.content.status_msg.clone(),
            Err(_) => Some(String::new()),
        };

        let last_active_ago = UInt::new(0);
        let currently_active = *new_state == PresenceState::Online;
        self.set_presence(
            user_id,
            new_state,
            Some(currently_active),
            last_active_ago,
            status_msg,
        )
        .await
    }

    /// Adds a presence event which will be saved until a new event replaces it.
    pub async fn set_presence(
        &self,
        user_id: &UserId,
        state: &PresenceState,
        currently_active: Option<bool>,
        last_active_ago: Option<UInt>,
        status_msg: Option<String>,
    ) -> Result<()> {
        let presence_state = match state.as_str() {
            "" => &PresenceState::Offline,
            &_ => state,
        };

        self.db
            .set_presence(
                user_id,
                presence_state,
                currently_active,
                last_active_ago,
                status_msg,
            )
            .await?;

        if (self.timeout_remote_users || self.services.server_state.user_is_local(user_id))
            && user_id != self.services.server_state.server_user
        {
            let timeout = match presence_state {
                PresenceState::Online => self.services.server.config.presence_idle_timeout_s,
                _ => self.services.server.config.presence_offline_timeout_s,
            };

            self.timer_sender
                .send((user_id.to_owned(), Duration::from_secs(timeout)))
                .map_err(|e| err!(Database("Failed to add presence timer: {e}")))?;
        }

        Ok(())
    }

    /// Removes the presence record for the given user from the database.
    ///
    /// TODO: Why is this not used?
    #[allow(dead_code)]
    pub async fn remove_presence(&self, user_id: &UserId) -> Result<()> {
        self.db.remove_presence(user_id).await
    }

    pub async fn unset_all_presence(&self) {
        let _cork = self.services.db.engine.cork_and_flush();

        for user_id in &self
            .services
            .users
            .list_local_users()
            .map(UserId::to_owned)
            .collect::<Vec<_>>()
            .await
        {
            let presence = self.db.get_presence(user_id).await;

            let presence = match presence {
                Ok((_, ref presence)) => &presence.content,
                _ => continue,
            };

            if !matches!(
                presence.presence,
                PresenceState::Unavailable | PresenceState::Online
            ) {
                trace!(?user_id, ?presence, "Skipping user");
                continue;
            }

            trace!(?user_id, ?presence, "Resetting presence to offline");

            _ = self
                .set_presence(
                    user_id,
                    &PresenceState::Offline,
                    Some(false),
                    presence.last_active_ago,
                    presence.status_msg.clone(),
                )
                .await
                .inspect_err(|e| {
                    debug_warn!(
                        ?presence,
                        "{user_id} has invalid presence in database and failed to reset it to \
						 offline: {e}"
                    );
                });
        }
    }

    /// Returns the most recent presence updates that happened after the event
    /// with id `since`.
    pub fn presence_since(
        &self,
        since: u64,
    ) -> impl Stream<Item = (&UserId, u64, &[u8])> + Send + '_ {
        self.db.presence_since(since)
    }

    #[inline]
    pub async fn from_json_bytes_to_event(
        &self,
        bytes: &[u8],
        user_id: &UserId,
    ) -> Result<PresenceEvent> {
        let presence = Presence::from_json_bytes(bytes)?;
        let event = presence
            .to_presence_event(user_id, &self.services.users)
            .await;

        Ok(event)
    }

    async fn process_presence_timer(&self, user_id: &OwnedUserId) -> Result<()> {
        let mut presence_state = PresenceState::Offline;
        let mut last_active_ago = None;
        let mut status_msg = None;

        let presence_event = self.get_presence(user_id).await;

        if let Ok(presence_event) = presence_event {
            presence_state = presence_event.content.presence;
            last_active_ago = presence_event.content.last_active_ago;
            status_msg = presence_event.content.status_msg;
        }

        let new_state = match (&presence_state, last_active_ago.map(u64::from)) {
            (PresenceState::Online, Some(ago)) if ago >= self.idle_timeout => {
                Some(PresenceState::Unavailable)
            }
            (PresenceState::Unavailable, Some(ago)) if ago >= self.offline_timeout => {
                Some(PresenceState::Offline)
            }
            _ => None,
        };

        debug!(
            "Processed presence timer for user '{user_id}': Old state = {presence_state}, New \
			 state = {new_state:?}"
        );

        if let Some(new_state) = new_state {
            self.set_presence(
                user_id,
                &new_state,
                Some(false),
                last_active_ago,
                status_msg,
            )
            .await?;
        }

        Ok(())
    }
}

async fn presence_timer(user_id: OwnedUserId, timeout: Duration) -> OwnedUserId {
    sleep(timeout).await;

    user_id
}
