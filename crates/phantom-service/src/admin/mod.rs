mod execute;
#[cfg(test)]
mod tests;

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
};

use async_trait::async_trait;
use phantom_core::{
    Result, debug, err, error, implement, info, matrix::pdu::PduEvent, result::LogErr,
    server::Server,
};
use ruma::{
    OwnedEventId, OwnedRoomId, RoomId, UserId, events::room::message::RoomMessageEventContent,
};
use tokio::sync::{Mutex, Notify, broadcast::error::RecvError, mpsc};

use crate::{Dep, rooms, server_state};

pub struct Service {
    services: Services,

    sender: mpsc::Sender<CommandInput>,

    receiver: Mutex<mpsc::Receiver<CommandInput>>,

    interrupt: Notify,

    processor: RwLock<Option<Processor>>,

    completer: RwLock<Option<Completer>>,
}

struct Services {
    server: Arc<Server>,
    alias: Dep<rooms::alias::Service>,
    server_state: Dep<server_state::Service>,
    state_cache: Dep<rooms::state_cache::Service>,

    services: RwLock<Option<Weak<crate::Services>>>,
}

#[derive(Debug)]
pub struct CommandInput {
    pub command: String,
    pub reply_id: Option<OwnedEventId>,
}

pub type Completer = fn(&str) -> String;

pub type Processor = fn(Arc<crate::Services>, CommandInput) -> ProcessorFuture;

pub type ProcessorFuture = Pin<Box<dyn Future<Output = ProcessorResult> + Send>>;

pub type ProcessorResult = Result<Option<CommandOutput>, CommandOutput>;

pub type CommandOutput = RoomMessageEventContent;

const COMMAND_QUEUE_LIMIT: usize = 512;

#[async_trait]
impl crate::Service for Service {
    fn build(args: crate::Args<'_>) -> Result<Arc<Self>> {
        let (sender, receiver) = mpsc::channel(COMMAND_QUEUE_LIMIT);

        Ok(Arc::new(Self {
            services: Services {
                server: args.server.clone(),
                server_state: args.depend::<server_state::Service>("server_state"),
                alias: args.depend::<rooms::alias::Service>("rooms::alias"),
                state_cache: args.depend::<rooms::state_cache::Service>("rooms::state_cache"),
                services: RwLock::new(None),
            },
            sender,
            receiver: Mutex::new(receiver),
            interrupt: Notify::new(),
            processor: RwLock::new(None),
            completer: RwLock::new(None),
        }))
    }

    async fn worker(self: Arc<Self>) -> Result {
        let mut receiver = self.receiver.lock().await;
        let mut signals = self.services.server.signal.subscribe();

        self.startup_execute().await?;

        loop {
            tokio::select! {
                () = self.interrupt.notified() => break,
                command = receiver.recv() => match command {
                    Some(command) => self.handle_command(command).await,
                    None => break,
                },
                signal = signals.recv() => match signal {
                    Ok(signal) => self.handle_signal(signal).await,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
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

#[implement(Service)]
pub fn command(&self, command: String, reply_id: Option<OwnedEventId>) -> Result {
    self.sender
        .try_send(CommandInput { command, reply_id })
        .map_err(|e| err!("Failed to enqueue admin command: {e}"))
}

#[implement(Service)]
pub async fn command_in_place(
    &self,
    command: String,
    reply_id: Option<OwnedEventId>,
) -> ProcessorResult {
    self.process_command(CommandInput { command, reply_id })
        .await
}

#[implement(Service)]
pub fn complete_command(&self, command: &str) -> Option<String> {
    self.completer
        .read()
        .expect("locked for reading")
        .map(|complete| complete(command))
}

#[implement(Service)]
pub fn set_processor(&self, processor: Option<Processor>) {
    *self.processor.write().expect("locked for writing") = processor;
}

#[implement(Service)]
pub fn set_completer(&self, completer: Option<Completer>) {
    *self.completer.write().expect("locked for writing") = completer;
}

#[implement(Service)]
pub(crate) fn set_services(&self, services: Option<&Arc<crate::Services>>) {
    *self.services.services.write().expect("locked for writing") = services.map(Arc::downgrade);
}

#[implement(Service)]
async fn process_command(&self, command: CommandInput) -> ProcessorResult {
    let processor = *self.processor.read().expect("locked for reading");

    let Some(processor) = processor else {
        return Err(CommandOutput::text_plain(
            "No admin command processor is registered; this build defines no admin commands.",
        ));
    };

    let services = self
        .services
        .services
        .read()
        .expect("locked for reading")
        .as_ref()
        .and_then(Weak::upgrade);

    let Some(services) = services else {
        return Err(CommandOutput::text_plain(
            "The server is not running commands: the services are still starting, or have \
             already stopped.",
        ));
    };

    processor(services, command).await
}

#[implement(Service)]
async fn handle_command(&self, command: CommandInput) {
    let reply_id = command.reply_id.clone();

    match self.process_command(command).await {
        Ok(None) => debug!(?reply_id, "Command successful with no response"),
        Ok(Some(output)) => info!(?reply_id, "Command successful:\n{}", output.body()),
        Err(output) => error!(?reply_id, "Command failed:\n{}", output.body()),
    }
}

#[implement(Service)]
async fn handle_signal(&self, signal: &'static str) {
    if signal == execute::SIGNAL {
        self.signal_execute().await.log_err().ok();
    }
}

#[implement(Service)]
pub async fn user_is_admin(&self, user_id: &UserId) -> bool {
    let Ok(admin_room) = self.get_admin_room().await else {
        return false;
    };

    self.services
        .state_cache
        .is_joined(user_id, &admin_room)
        .await
}

#[implement(Service)]
pub async fn get_admin_room(&self) -> Result<OwnedRoomId> {
    let admin_alias = &self.services.server_state.admin_alias;
    let room_id = self.services.alias.resolve_local_alias(admin_alias).await?;

    self.services
        .state_cache
        .is_joined(&self.services.server_state.server_user, &room_id)
        .await
        .then_some(room_id)
        .ok_or_else(|| err!(Request(NotFound("Admin user not joined to admin room"))))
}

#[implement(Service)]
pub async fn is_admin_room(&self, room_id: &RoomId) -> bool {
    self.get_admin_room()
        .await
        .is_ok_and(|admin_room| admin_room == *room_id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Invocation {
    Direct,

    Escaped,
}

fn invocation(body: &str, server_user: &UserId) -> Option<Invocation> {
    if let Some(escaped) = body.strip_prefix('\\') {
        return escaped
            .trim_start_matches('\\')
            .starts_with("!admin")
            .then_some(Invocation::Escaped);
    }

    (body.starts_with("!admin") || body.starts_with(server_user.as_str()))
        .then_some(Invocation::Direct)
}

#[implement(Service)]
pub async fn is_admin_command(&self, pdu: &PduEvent, body: &str) -> bool {
    let server_user = &self.services.server_state.server_user;

    let Some(invocation) = invocation(body, server_user) else {
        return false;
    };

    let Ok(admin_room) = self.get_admin_room().await else {
        return false;
    };

    let in_admin_room = admin_room == pdu.room_id;

    match invocation {
        Invocation::Direct if !in_admin_room => return false,

        Invocation::Escaped
            if !self.services.server_state.user_is_local(&pdu.sender)
                || !self.services.server.config.admin.admin_escape_commands =>
        {
            return false;
        }

        Invocation::Direct | Invocation::Escaped => {}
    }

    if !self
        .services
        .state_cache
        .is_joined(&pdu.sender, &admin_room)
        .await
    {
        return false;
    }

    let is_recovery = self
        .services
        .server
        .config
        .admin
        .emergency_password
        .is_some();
    if in_admin_room && pdu.sender == *server_user && !is_recovery {
        return false;
    }

    true
}
