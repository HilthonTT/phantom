//! The `!admin` commands of the admin room. Each one runs the same handler as
//! the admin API route the console calls, so the two cannot drift apart, and
//! renders the handler's JSON as a notice.

mod render;
mod words;

#[cfg(test)]
mod tests;

use std::{future::Future, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use clap::{Parser, Subcommand, error::ErrorKind};
use phantom_core::{Err, Result, err};
use phantom_service::{
    Services,
    ops::admin::{CommandInput, CommandOutput, ProcessorFuture, ProcessorResult},
};
use ruma::{
    OwnedDeviceId, OwnedMxcUri, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, OwnedUserId,
    RoomAliasId, UserId,
};
use serde_json::{Value, json};

use self::render::{Column, done, error, fields, table};
use super::{appservices, insight, ops, rooms, settings, stats, tokens, users};
use crate::router::AdminAuth;

/// Registered with the admin service at startup; runs one command.
pub fn processor(services: Arc<Services>, input: CommandInput) -> ProcessorFuture {
    Box::pin(async move { process(services, input).await })
}

/// Completes the command a console prompt has started, by subcommand name.
#[must_use]
pub fn complete(line: &str) -> String {
    let mut words = words::split(line).unwrap_or_default();
    let partial = if line.ends_with(char::is_whitespace) {
        String::new()
    } else {
        words.pop().unwrap_or_default()
    };

    let mut command = <Cli as clap::CommandFactory>::command();
    for word in words.iter().skip(1) {
        match command.find_subcommand(word) {
            Some(sub) => command = sub.clone(),
            None => return line.to_owned(),
        }
    }

    let matches: Vec<&str> = command
        .get_subcommands()
        .map(clap::Command::get_name)
        .filter(|name| name.starts_with(&partial))
        .collect();

    match matches.as_slice() {
        [only] => {
            let prefix = line.strip_suffix(partial.as_str()).unwrap_or(line);

            format!("{prefix}{only} ")
        }
        _ => line.to_owned(),
    }
}

async fn process(services: Arc<Services>, input: CommandInput) -> ProcessorResult {
    let words = match words::split(&input.command) {
        Ok(words) => words,
        Err(e) => return Err(error(&e)),
    };

    let cli = match Cli::try_parse_from(words::argv(words, &services.server_state.server_user)) {
        Ok(cli) => cli,
        Err(e) => {
            let text = e.render().to_string();

            return match e.kind() {
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayVersion
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    Ok(Some(render::preformatted(&text)))
                }
                _ => Err(render::preformatted(&text)),
            };
        }
    };

    let admin = AdminAuth {
        user_id: input
            .sender
            .unwrap_or_else(|| services.server_state.server_user.clone()),
        device_id: OwnedDeviceId::from(COMMAND_DEVICE),
    };

    run(&services, admin, cli.command)
        .await
        .map(Some)
        .map_err(|e| error(&e.message()))
}

/// The device an admin acts from in the admin room. No real device has it, so
/// the guards that spare the admin's own device spare none here.
const COMMAND_DEVICE: &str = "!admin";

#[derive(Debug, Parser)]
#[command(
    name = "!admin",
    about = "Administer this server from the admin room.",
    disable_version_flag = true,
    help_template = "{about}\n\n{usage-heading} {usage}\n\n{all-args}"
)]
pub(super) struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Accounts on this server.
    #[command(subcommand)]
    Users(UserCommand),

    /// Rooms this server knows of.
    #[command(subcommand)]
    Rooms(RoomCommand),

    /// Registration tokens.
    #[command(subcommand)]
    Tokens(TokenCommand),

    /// Stored media.
    #[command(subcommand)]
    Media(MediaCommand),

    /// The servers this one federates with.
    #[command(subcommand)]
    Federation(FederationCommand),

    /// Abuse reports.
    #[command(subcommand)]
    Reports(ReportCommand),

    /// Registered appservices.
    #[command(subcommand)]
    Appservices(AppserviceCommand),

    /// The server itself.
    #[command(subcommand)]
    Server(ServerCommand),

    /// Long operations: room shutdowns and deletions, backups.
    #[command(subcommand)]
    Tasks(TaskCommand),
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// The long operations of the last week.
    List,

    /// Asks a running task to stop.
    Cancel { id: String },
}

#[derive(Debug, Subcommand)]
enum UserCommand {
    /// Lists every local account.
    List,

    /// Shows one account.
    Info { user: String },

    /// Creates a local account, with a random password unless one is given.
    Create {
        user: String,
        password: Option<String>,
    },

    /// Lists an account's devices.
    Devices { user: String },

    /// Signs one of an account's devices out.
    DeleteDevice {
        user: String,
        device_id: OwnedDeviceId,
    },

    /// Sets an account's password, random unless given, and signs it out.
    ResetPassword {
        user: String,
        password: Option<String>,

        /// Leave the account's devices signed in.
        #[arg(long)]
        keep_sessions: bool,
    },

    /// Deactivates an account: signs it out and makes it leave every room.
    Deactivate {
        user: String,

        /// Also erase its data and profile for good.
        #[arg(long)]
        erase: bool,
    },

    /// Lifts a deactivation, with a random password unless one is given.
    /// Rooms, profile and admin rights do not come back.
    Reactivate {
        user: String,
        password: Option<String>,
    },

    /// Makes a user an admin by joining them to this room.
    MakeAdmin { user: String },

    /// Takes a user's admin rights away.
    RevokeAdmin { user: String },
}

#[derive(Debug, Subcommand)]
enum RoomCommand {
    /// Lists every room the server knows of.
    List,

    /// Shows one room, by ID or local alias.
    Info { room: OwnedRoomOrAliasId },

    /// Bans a room: local users can no longer join and federation stops.
    Ban { room: OwnedRoomOrAliasId },

    /// Lifts a room's ban.
    Unban { room: OwnedRoomOrAliasId },

    /// Evicts every local member and frees the room's aliases, as a task.
    Shutdown { room: OwnedRoomOrAliasId },

    /// Shuts the room down and purges everything held of it, as a task.
    Delete {
        room: OwnedRoomOrAliasId,

        /// Purge even what the room's state cannot account for.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TokenCommand {
    /// Lists the tokens registration accepts.
    List,

    /// Creates a token, random unless given.
    Create {
        token: Option<String>,

        /// How many registrations it allows; unlimited by default.
        #[arg(long)]
        uses: Option<u64>,

        /// How many seconds it stays valid; forever by default.
        #[arg(long)]
        expires_in: Option<u64>,
    },

    /// Revokes a token.
    Revoke { token: String },
}

#[derive(Debug, Subcommand)]
enum MediaCommand {
    /// Lists the stored media.
    List,

    /// Deletes one file, by its mxc:// URI.
    Delete { mxc: OwnedMxcUri },
}

#[derive(Debug, Subcommand)]
enum FederationCommand {
    /// Lists the servers this one shares rooms with or has contacted.
    List,

    /// Deletes the cached media of a remote server.
    PurgeMedia { server: OwnedServerName },
}

#[derive(Debug, Subcommand)]
enum ReportCommand {
    /// Lists the open abuse reports.
    List,

    /// Dismisses a report.
    Dismiss { id: String },
}

#[derive(Debug, Subcommand)]
enum AppserviceCommand {
    /// Lists the registered appservices.
    List,
}

#[derive(Debug, Subcommand)]
enum ServerCommand {
    /// Uptime, users, rooms and storage.
    Status,

    /// The service workers and their state.
    Services,

    /// The running configuration, secrets masked; only lines containing the
    /// filter when one is given.
    Config { filter: Option<String> },

    /// The recent log, newest first.
    Logs {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// The long operations of the last week.
    Tasks,

    /// Reads the config and TLS certificate files again.
    Reload,

    /// Backs the database up into database_backup_path, as a task.
    Backup,

    /// Restarts the server process.
    Restart,

    /// Stops the server.
    Shutdown,
}

async fn run(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: Command,
) -> Result<CommandOutput> {
    match command {
        Command::Users(command) => run_users(services, admin, command).await,
        Command::Rooms(command) => run_rooms(services, admin, command).await,
        Command::Tokens(command) => run_tokens(services, admin, command).await,
        Command::Media(command) => run_media(services, admin, command).await,
        Command::Federation(command) => run_federation(services, admin, command).await,
        Command::Reports(command) => run_reports(services, admin, command).await,
        Command::Appservices(AppserviceCommand::List) => {
            let rows = call(appservices::appservices(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("id", "id"),
                    Column::new("sender", "sender_localpart"),
                    Column::new("url", "url"),
                ],
            ))
        }
        Command::Server(command) => run_server(services, admin, command).await,
        Command::Tasks(command) => run_tasks(services, admin, command).await,
    }
}

async fn run_tasks(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: TaskCommand,
) -> Result<CommandOutput> {
    match command {
        TaskCommand::List => {
            let rows = call(ops::tasks(state(services), admin)).await?;

            Ok(table(&rows, &task_columns()))
        }
        TaskCommand::Cancel { id } => {
            let task = call(ops::cancel_task(state(services), admin, Path(id.clone()))).await?;

            Ok(match task["status"].as_str() {
                Some("cancelled") => done(&format!("Cancelled task {id}.")),
                _ => done(&format!(
                    "Asked task {id} to stop; it stops at its next safe point. `!admin tasks \
                     list` shows when it has."
                )),
            })
        }
    }
}

fn task_columns() -> [Column; 6] {
    [
        Column::new("task", "id"),
        Column::new("action", "action"),
        Column::new("resource", "resource"),
        Column::new("status", "status"),
        Column::time("updated", "updated_at_ms"),
        Column::new("error", "error"),
    ]
}

async fn run_users(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: UserCommand,
) -> Result<CommandOutput> {
    match command {
        UserCommand::List => {
            let rows = call(users::users(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("user", "user_id"),
                    Column::new("name", "display_name"),
                    Column::new("admin", "admin"),
                    Column::new("deactivated", "deactivated"),
                    Column::new("devices", "devices"),
                    Column::new("rooms", "rooms_joined"),
                ],
            ))
        }
        UserCommand::Info { user } => {
            let user_id = user_id(services, &user)?;
            let rows = call(users::users(state(services), admin)).await?;

            let row = rows
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["user_id"] == user_id.as_str())
                .ok_or_else(|| err!(Request(NotFound("{user_id} is not a local account."))))?;

            Ok(fields(row))
        }
        UserCommand::Create { user, password } => {
            let user_id = user_id(services, &user)?;
            let generated = password.is_none();
            let password = password.unwrap_or_else(|| phantom_core::rand::string(PASSWORD_LENGTH));

            crate::client::full_register(services, &user_id, Some(&password)).await?;

            Ok(match generated {
                true => done(&format!("Created {user_id} with the password: {password}")),
                false => done(&format!("Created {user_id}.")),
            })
        }
        UserCommand::Devices { user } => {
            let user_id = user_id(services, &user)?;
            let rows = call(users::devices(state(services), admin)).await?;
            let rows: Value = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| row["user_id"] == user_id.as_str())
                .cloned()
                .collect();

            Ok(table(
                &rows,
                &[
                    Column::new("device", "device_id"),
                    Column::new("name", "display_name"),
                    Column::new("last ip", "last_seen_ip"),
                    Column::time("last seen", "last_seen_ms"),
                ],
            ))
        }
        UserCommand::DeleteDevice { user, device_id } => {
            let user_id = user_id(services, &user)?;
            call(users::delete_device(
                state(services),
                admin,
                Path((user_id, device_id)),
            ))
            .await?;

            Ok(done("Signed the device out."))
        }
        UserCommand::ResetPassword {
            user,
            password,
            keep_sessions,
        } => {
            let user_id = user_id(services, &user)?;
            let generated = password.is_none();
            let password = password.unwrap_or_else(|| phantom_core::rand::string(PASSWORD_LENGTH));
            let body = json!({ "password": password, "logout_devices": !keep_sessions });

            call(users::password(
                state(services),
                admin,
                Path(user_id.clone()),
                bytes(&body),
            ))
            .await?;

            Ok(match generated {
                true => done(&format!("Set {user_id}'s password to: {password}")),
                false => done(&format!("Set {user_id}'s password.")),
            })
        }
        UserCommand::Deactivate { user, erase } => {
            let user_id = user_id(services, &user)?;
            let body = json!({ "erase": erase });

            call(users::deactivate(
                state(services),
                admin,
                Path(user_id.clone()),
                bytes(&body),
            ))
            .await?;

            Ok(done(&format!("Deactivated {user_id}.")))
        }
        UserCommand::Reactivate { user, password } => {
            let user_id = user_id(services, &user)?;
            let generated = password.is_none();
            let password = password.unwrap_or_else(|| phantom_core::rand::string(PASSWORD_LENGTH));

            call(users::reactivate(
                state(services),
                admin,
                Path(user_id.clone()),
                bytes(&json!({ "password": password })),
            ))
            .await?;

            Ok(match generated {
                true => done(&format!(
                    "Reactivated {user_id} with the password: {password}"
                )),
                false => done(&format!("Reactivated {user_id}.")),
            })
        }
        UserCommand::MakeAdmin { user } => {
            let user_id = user_id(services, &user)?;
            call(users::grant_admin(
                state(services),
                admin,
                Path(user_id.clone()),
            ))
            .await?;

            Ok(done(&format!("{user_id} is now an admin.")))
        }
        UserCommand::RevokeAdmin { user } => {
            let user_id = user_id(services, &user)?;
            call(users::revoke_admin(
                state(services),
                admin,
                Path(user_id.clone()),
            ))
            .await?;

            Ok(done(&format!("{user_id} is no longer an admin.")))
        }
    }
}

async fn run_rooms(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: RoomCommand,
) -> Result<CommandOutput> {
    match command {
        RoomCommand::List => {
            let rows = call(rooms::rooms(state(services), admin)).await?;

            Ok(table(&rows, &room_columns()))
        }
        RoomCommand::Info { room } => {
            let room_id = room_id(services, &room).await?;
            let rows = call(rooms::rooms(state(services), admin)).await?;

            let row = rows
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["room_id"] == room_id.as_str())
                .ok_or_else(|| err!(Request(NotFound("The server does not know {room_id}."))))?;

            Ok(fields(row))
        }
        RoomCommand::Ban { room } => {
            let room_id = room_id(services, &room).await?;
            call(rooms::ban(state(services), admin, Path(room_id.clone()))).await?;

            Ok(done(&format!("Banned {room_id}.")))
        }
        RoomCommand::Unban { room } => {
            let room_id = room_id(services, &room).await?;
            call(rooms::unban(state(services), admin, Path(room_id.clone()))).await?;

            Ok(done(&format!("Lifted the ban on {room_id}.")))
        }
        RoomCommand::Shutdown { room } => {
            let room_id = room_id(services, &room).await?;
            let task = call(rooms::shutdown(
                state(services),
                admin,
                Path(room_id.clone()),
            ))
            .await?;

            Ok(task_started(&task, "Shutting", &room_id))
        }
        RoomCommand::Delete { room, force } => {
            let room_id = room_id(services, &room).await?;
            let body = json!({ "force": force });
            let task = call(rooms::delete(
                state(services),
                admin,
                Path(room_id.clone()),
                bytes(&body),
            ))
            .await?;

            Ok(task_started(&task, "Deleting", &room_id))
        }
    }
}

fn room_columns() -> [Column; 6] {
    [
        Column::new("room", "room_id"),
        Column::new("name", "name"),
        Column::new("alias", "canonical_alias"),
        Column::new("version", "version"),
        Column::new("members", "joined_members"),
        Column::new("banned", "banned"),
    ]
}

fn task_started(task: &Value, doing: &str, room_id: &OwnedRoomId) -> CommandOutput {
    let task_id = task["task_id"].as_str().unwrap_or("");

    done(&format!(
        "{doing} {room_id} as task {task_id}; `!admin tasks list` shows how it went, and \
         `!admin tasks cancel {task_id}` stops it."
    ))
}

async fn run_tokens(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: TokenCommand,
) -> Result<CommandOutput> {
    match command {
        TokenCommand::List => {
            let rows = call(tokens::tokens(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("token", "token"),
                    Column::new("source", "source"),
                    Column::new("uses", "uses"),
                    Column::new("max uses", "max_uses"),
                    Column::time("expires", "expires_at_ms"),
                ],
            ))
        }
        TokenCommand::Create {
            token,
            uses,
            expires_in,
        } => {
            let body = json!({
                "token": token,
                "uses_allowed": uses,
                "expires_in_secs": expires_in,
            });
            let created = call(tokens::create(state(services), admin, bytes(&body))).await?;
            let token = created["token"].as_str().unwrap_or("");

            Ok(done(&format!("Created the registration token: {token}")))
        }
        TokenCommand::Revoke { token } => {
            call(tokens::revoke(state(services), admin, Path(token))).await?;

            Ok(done("Revoked the token."))
        }
    }
}

async fn run_media(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: MediaCommand,
) -> Result<CommandOutput> {
    match command {
        MediaCommand::List => {
            let rows = call(insight::media(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("mxc", "mxc"),
                    Column::new("type", "content_type"),
                    Column::new("bytes", "size"),
                    Column::new("uploader", "uploader"),
                    Column::time("stored", "created_at_ms"),
                ],
            ))
        }
        MediaCommand::Delete { mxc } => {
            let parts = mxc
                .parts()
                .map_err(|e| err!(Request(InvalidParam("{mxc} is not an mxc:// URI: {e}"))))?;
            let (server_name, media_id) = parts;
            let path = Path((server_name.to_string(), media_id.to_owned()));

            call(insight::delete_media(state(services), admin, path)).await?;

            Ok(done(&format!("Deleted {mxc}.")))
        }
    }
}

async fn run_federation(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: FederationCommand,
) -> Result<CommandOutput> {
    match command {
        FederationCommand::List => {
            let rows = call(insight::federation(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("server", "server"),
                    Column::new("rooms", "rooms"),
                    Column::time("last contact", "last_contact_ms"),
                    Column::new("sending", "sending"),
                    Column::new("backoff", "backoff"),
                ],
            ))
        }
        FederationCommand::PurgeMedia { server } => {
            let removed = call(insight::purge_remote_media(
                state(services),
                admin,
                Path(server.clone()),
            ))
            .await?;

            Ok(done(&format!(
                "Purged {server}'s media: {}",
                render::scalar(&removed)
            )))
        }
    }
}

async fn run_reports(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: ReportCommand,
) -> Result<CommandOutput> {
    match command {
        ReportCommand::List => {
            let rows = call(insight::reports(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("id", "id"),
                    Column::new("kind", "kind"),
                    Column::new("reporter", "reporter"),
                    Column::new("room", "room_id"),
                    Column::new("event", "event_id"),
                    Column::new("user", "user_id"),
                    Column::new("reason", "reason"),
                    Column::time("at", "at_ms"),
                ],
            ))
        }
        ReportCommand::Dismiss { id } => {
            call(insight::dismiss_report(state(services), admin, Path(id))).await?;

            Ok(done("Dismissed the report."))
        }
    }
}

async fn run_server(
    services: &Arc<Services>,
    admin: AdminAuth,
    command: ServerCommand,
) -> Result<CommandOutput> {
    match command {
        ServerCommand::Status => Ok(fields(&call(stats::stats(state(services), admin)).await?)),
        ServerCommand::Services => {
            let rows = call(insight::services(state(services), admin)).await?;

            Ok(table(
                &rows,
                &[
                    Column::new("service", "name"),
                    Column::new("status", "status"),
                    Column::new("restarts", "restarts"),
                    Column::new("error", "error"),
                ],
            ))
        }
        ServerCommand::Config { filter } => {
            let rows = call(settings::settings(state(services), admin)).await?;
            let rows: Value = rows
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| {
                    filter
                        .as_deref()
                        .is_none_or(|filter| row.to_string().contains(filter))
                })
                .cloned()
                .collect();

            Ok(table(
                &rows,
                &[Column::new("key", "key"), Column::new("value", "value")],
            ))
        }
        ServerCommand::Logs { limit } => {
            let query = Query(serde_json::from_value(json!({ "limit": limit }))?);
            let rows = call(insight::logs(state(services), admin, query)).await?;

            Ok(table(
                &rows,
                &[
                    Column::time("at", "at_ms"),
                    Column::new("level", "level"),
                    Column::new("target", "target"),
                    Column::new("message", "message"),
                ],
            ))
        }
        ServerCommand::Tasks => {
            let rows = call(ops::tasks(state(services), admin)).await?;

            Ok(table(&rows, &task_columns()))
        }
        ServerCommand::Reload => {
            call(ops::reload(state(services), admin)).await?;

            Ok(done("Reloaded the config."))
        }
        ServerCommand::Backup => {
            let task = call(ops::backup(state(services), admin)).await?;
            let task_id = task["task_id"].as_str().unwrap_or("");

            Ok(done(&format!(
                "Backing the database up as task {task_id}; `!admin server tasks` shows how it \
                 went."
            )))
        }
        ServerCommand::Restart => {
            services.server.restart()?;

            Ok(done("Restarting."))
        }
        ServerCommand::Shutdown => {
            services.server.shutdown()?;

            Ok(done("Shutting down."))
        }
    }
}

const PASSWORD_LENGTH: usize = 24;

fn state(services: &Arc<Services>) -> State<crate::router::State> {
    State(crate::router::State::new(services.clone()))
}

fn bytes(body: &Value) -> Bytes {
    Bytes::from(body.to_string())
}

/// Runs an admin API handler and reads back the JSON it answers with.
async fn call<F, R>(handler: F) -> Result<Value>
where
    F: Future<Output = Result<R>>,
    R: IntoResponse,
{
    let response = handler.await?.into_response();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| err!("Could not read the handler's answer: {e}"))?;

    if body.is_empty() {
        return Ok(Value::Null);
    }

    Ok(serde_json::from_slice(&body)?)
}

/// A user ID, or a bare localpart on this server.
fn user_id(services: &Services, user: &str) -> Result<OwnedUserId> {
    let user = user.strip_prefix('@').unwrap_or(user);
    let user = match user.contains(':') {
        true => format!("@{user}"),
        false => format!("@{user}:{}", services.server_state.server_name()),
    };

    UserId::parse(&user).map_err(|e| err!(Request(InvalidParam("{user} is not a user ID: {e}"))))
}

/// A room ID, or the room a local alias points to.
async fn room_id(services: &Services, room: &OwnedRoomOrAliasId) -> Result<OwnedRoomId> {
    match OwnedRoomId::try_from(room.clone()) {
        Ok(room_id) => Ok(room_id),
        Err(alias) => {
            let alias: &RoomAliasId = &alias;
            if !services.server_state.server_is_ours(alias.server_name()) {
                return Err!(Request(InvalidParam(
                    "{alias} is not a local alias; give the room's ID."
                )));
            }

            services
                .rooms
                .alias
                .resolve_local_alias(alias)
                .await
                .map_err(|_| err!(Request(NotFound("No room has the alias {alias}."))))
        }
    }
}
