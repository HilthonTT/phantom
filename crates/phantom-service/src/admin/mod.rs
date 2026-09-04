//! The admin room, and the commands that arrive through it.
//!
//! The server keeps one room as its console. Whoever is joined to that room is
//! an administrator of this server — the membership *is* the privilege, which
//! is why granting admin is an invite and why losing it is a leave — and a
//! message in it addressed to the server user is a command.
//!
//! Commands are not defined here. This service owns the room, the privilege
//! check and the queue, and hands each command to a [`Processor`] registered
//! by whatever module defines the command set. Nothing registers one yet, so
//! the queue is fed only by [`execute`] and answered with a message saying so.
//! The indirection is what lets the command set be replaced — or reloaded —
//! without the queue and the room lookup moving with it.
//!
//! What is deliberately not here: delivering a command's output back into the
//! room, granting a user admin, and creating the admin room in the first
//! place. All three append PDUs, and phantom has not ported the timeline write
//! path yet; they are parked in `pending_write_path.rs` beside this file,
//! which is not a module. Until that lands, command output goes to the log.

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

    /// Commands waiting for the worker to pick them up.
    sender: mpsc::Sender<CommandInput>,

    /// Held behind a lock because the worker takes it and the trait hands the
    /// worker a shared reference; a second worker would be a bug, and this is
    /// what makes it one that cannot happen.
    receiver: Mutex<mpsc::Receiver<CommandInput>>,

    /// Signalled by [`crate::Service::interrupt`], which is not async and so
    /// cannot reach the receiver to close it.
    interrupt: Notify,

    /// What actually runs a command, once something defines the commands.
    ///
    /// A plain function pointer rather than a boxed closure: it is `Copy`, so
    /// a caller reads it out and drops the lock before awaiting, which is what
    /// keeps the guard from having to be held across the whole command.
    processor: RwLock<Option<Processor>>,

    /// Tab-completion for the same command set, registered alongside it.
    completer: RwLock<Option<Completer>>,
}

struct Services {
    server: Arc<Server>,
    alias: Dep<rooms::alias::Service>,
    server_state: Dep<server_state::Service>,
    state_cache: Dep<rooms::state_cache::Service>,

    /// The whole service graph, which is what a command runs against.
    ///
    /// Weak, and set after every service is built rather than at build time:
    /// the graph is not complete until this service is in it, and a strong
    /// reference from a service to the collection holding it never drops.
    services: RwLock<Option<Weak<crate::Services>>>,
}

/// A command as it arrives: the text, and the event it is answering if it came
/// from a room.
#[derive(Debug)]
pub struct CommandInput {
    pub command: String,
    pub reply_id: Option<OwnedEventId>,
}

/// Completes a partially typed command. The output replaces the input buffer
/// whole rather than being appended to it.
pub type Completer = fn(&str) -> String;

/// Runs one command against the service graph.
pub type Processor = fn(Arc<crate::Services>, CommandInput) -> ProcessorFuture;

/// Return type of a [`Processor`].
pub type ProcessorFuture = Pin<Box<dyn Future<Output = ProcessorResult> + Send>>;

/// How a command finished. Both variants are finished messages that have
/// already digested whatever went wrong, so the wrapping only records whether
/// the command failed without the caller having to read the text. `Ok(None)`
/// is a command that succeeded with nothing to say.
pub type ProcessorResult = Result<Option<CommandOutput>, CommandOutput>;

/// What a command produces: a message, because the room is the console.
pub type CommandOutput = RoomMessageEventContent;

/// How many commands may be queued before [`Service::command`] starts
/// refusing them. Reaching it means commands are arriving faster than they run,
/// which is a caller to slow down rather than a queue to grow.
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

/// Queues a command for the worker and returns without waiting for it.
///
/// Errors when the queue is full, rather than blocking the caller until it
/// drains: this is called from the timeline, and a room that fills the queue
/// must not be able to stall the task appending its events.
#[implement(Service)]
pub fn command(&self, command: String, reply_id: Option<OwnedEventId>) -> Result {
    self.sender
        .try_send(CommandInput { command, reply_id })
        .map_err(|e| err!("Failed to enqueue admin command: {e}"))
}

/// Runs a command on the current task and waits for it to finish.
#[implement(Service)]
pub async fn command_in_place(
    &self,
    command: String,
    reply_id: Option<OwnedEventId>,
) -> ProcessorResult {
    self.process_command(CommandInput { command, reply_id })
        .await
}

/// Completes a partially typed command, or `None` where no completer is
/// registered.
#[implement(Service)]
pub fn complete_command(&self, command: &str) -> Option<String> {
    self.completer
        .read()
        .expect("locked for reading")
        .map(|complete| complete(command))
}

/// Registers what runs a command, or `None` to unregister it.
#[implement(Service)]
pub fn set_processor(&self, processor: Option<Processor>) {
    *self.processor.write().expect("locked for writing") = processor;
}

/// Registers the tab-completer for the registered command set, or `None` to
/// unregister it.
#[implement(Service)]
pub fn set_completer(&self, completer: Option<Completer>) {
    *self.completer.write().expect("locked for writing") = completer;
}

/// Points the service at the completed service graph, which is what commands
/// run against. Called once everything is built, and again with `None` on
/// shutdown.
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

/// Runs a queued command and disposes of whatever it produced.
///
/// Upstream turns the output into a reply in the room the command came from.
/// That is the timeline write path, which phantom has not ported yet, so the
/// output is logged — the same place the configured startup commands' output
/// goes — rather than dropped.
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

/// Whether `user_id` is an administrator of this server, which is to say
/// joined to the admin room.
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

/// The admin room, if there is one.
///
/// A room the server user has left is not the admin room any more, whatever
/// the alias still points at: the server posts the responses, so a room it is
/// not in cannot be a console. Errors are the database's, plus a not-found for
/// that case.
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

/// How a message asks for a command to be run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Invocation {
    /// `!admin …`, or the server user's own name, which is a command only in
    /// the admin room.
    Direct,

    /// `\!admin …`, which is a command wherever it is typed and is echoed
    /// there. It is how an admin answers a question in the room it was asked
    /// in.
    Escaped,
}

/// Reads what a message body names, and nothing else. Whether the sender may
/// run it is [`Service::is_admin_command`]'s to decide.
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

/// Whether an event is a command this server should run.
///
/// Called for every message the timeline appends, so the prefix is read first
/// and the room and membership lookups only happen for something that already
/// looks like a command. The admin room is then resolved once and answers all
/// three questions that follow — which room this is, whether the sender is an
/// admin, and whether the server is talking to itself.
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
                || !self.services.server.config.admin_escape_commands =>
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

    let is_recovery = self.services.server.config.emergency_password.is_some();
    if in_admin_room && pdu.sender == *server_user && !is_recovery {
        return false;
    }

    true
}
