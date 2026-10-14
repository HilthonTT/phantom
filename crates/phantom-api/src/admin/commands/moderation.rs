//! The `!admin` moderation commands: blocking servers, quarantining media and
//! purging old history and media. Each runs the admin API handler of
//! `admin::moderation`, as every other command does.

use std::sync::Arc;

use axum::extract::Path;
use clap::Args;
use phantom_core::{Err, Result, err, time::now_millis};
use phantom_service::{Services, ops::admin::CommandOutput};
use ruma::{OwnedEventId, OwnedMxcUri, OwnedRoomOrAliasId};
use serde_json::{Value, json};

use super::{
    bytes, call,
    render::{Column, done, scalar, table},
    room_id, state, user_id,
};
use crate::{admin::moderation, router::AdminAuth};

#[derive(Debug, Args)]
pub(super) struct BlockArgs {
    /// The server to block, or with --pattern a regular expression matched
    /// against server names as the config file's list is.
    target: String,

    /// Read the target as a regular expression rather than one server.
    #[arg(long)]
    pattern: bool,

    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct UnblockArgs {
    /// The server to unblock, or with --pattern the pattern to remove.
    target: String,

    /// Remove exactly this pattern rather than every pattern matching a
    /// server.
    #[arg(long)]
    pattern: bool,
}

#[derive(Debug, Args)]
pub(super) struct QuarantineArgs {
    mxc: OwnedMxcUri,

    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct QuarantineRoomArgs {
    room: OwnedRoomOrAliasId,

    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct QuarantineUserArgs {
    user: String,

    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct PurgeMediaArgs {
    /// Purge what was stored more than this many days ago.
    #[arg(long)]
    days: u64,
}

#[derive(Debug, Args)]
pub(super) struct PurgeHistoryArgs {
    room: OwnedRoomOrAliasId,

    /// Purge events sent more than this many days ago.
    #[arg(
        long,
        required_unless_present_any = ["before_event", "before_ts"],
        conflicts_with_all = ["before_event", "before_ts"]
    )]
    days: Option<u64>,

    /// Purge events placed before this one.
    #[arg(long, conflicts_with = "before_ts")]
    before_event: Option<OwnedEventId>,

    /// Purge events sent before this time, in milliseconds since the epoch.
    #[arg(long)]
    before_ts: Option<u64>,

    /// Purge the events local users sent too; they exist nowhere else.
    #[arg(long)]
    include_local: bool,
}

pub(super) async fn blocks(services: &Arc<Services>, admin: AdminAuth) -> Result<CommandOutput> {
    let rows = call(moderation::blocks(state(services), admin)).await?;

    Ok(table(
        &rows,
        &[
            Column::new("pattern", "pattern"),
            Column::new("source", "source"),
            Column::new("reason", "reason"),
            Column::new("by", "by"),
            Column::time("since", "blocked_at_ms"),
        ],
    ))
}

pub(super) async fn block(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: BlockArgs,
) -> Result<CommandOutput> {
    let body = match args.pattern {
        true => json!({ "pattern": args.target, "reason": args.reason }),
        false => json!({ "server": args.target, "reason": args.reason }),
    };

    let answer = call(moderation::block(state(services), admin, bytes(&body))).await?;
    let pattern = scalar(&answer["pattern"]);

    Ok(match answer["added"].as_bool() {
        Some(true) => done(&format!("Blocked servers matching {pattern}.")),
        _ => done(&format!("{pattern} was already blocked.")),
    })
}

pub(super) async fn unblock(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: UnblockArgs,
) -> Result<CommandOutput> {
    let body = match args.pattern {
        true => json!({ "pattern": args.target }),
        false => json!({ "server": args.target }),
    };

    let answer = call(moderation::unblock(state(services), admin, bytes(&body))).await?;
    let removed = list(&answer["removed"]);
    let still = list(&answer["still_blocked_by"]);

    Ok(match (removed.is_empty(), still.is_empty()) {
        (_, true) => done(&format!("Removed {removed} from the block list.")),
        (true, false) => done(&format!(
            "{} is blocked only by the config file ({still}); change it there.",
            args.target
        )),
        (false, false) => done(&format!(
            "Removed {removed}, but the config file still blocks it ({still})."
        )),
    })
}

pub(super) async fn quarantined(
    services: &Arc<Services>,
    admin: AdminAuth,
) -> Result<CommandOutput> {
    let rows = call(moderation::quarantined(state(services), admin)).await?;

    Ok(table(
        &rows,
        &[
            Column::new("mxc", "mxc"),
            Column::new("stored", "stored"),
            Column::new("reason", "reason"),
            Column::new("by", "by"),
            Column::time("since", "quarantined_at_ms"),
        ],
    ))
}

pub(super) async fn quarantine(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: QuarantineArgs,
) -> Result<CommandOutput> {
    let path = mxc_path(&args.mxc)?;
    let body = json!({ "reason": args.reason });
    let answer = call(moderation::quarantine(
        state(services),
        admin,
        path,
        bytes(&body),
    ))
    .await?;

    Ok(match answer["quarantined"].as_u64() {
        Some(0) => done(&format!("{} was already quarantined.", args.mxc)),
        _ => done(&format!("Quarantined {}.", args.mxc)),
    })
}

pub(super) async fn unquarantine(
    services: &Arc<Services>,
    admin: AdminAuth,
    mxc: OwnedMxcUri,
) -> Result<CommandOutput> {
    call(moderation::unquarantine(
        state(services),
        admin,
        mxc_path(&mxc)?,
    ))
    .await?;

    Ok(done(&format!("Lifted the quarantine of {mxc}.")))
}

pub(super) async fn quarantine_room(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: QuarantineRoomArgs,
) -> Result<CommandOutput> {
    let room_id = room_id(services, &args.room).await?;
    let body = json!({ "reason": args.reason });
    let task = call(moderation::quarantine_room_media(
        state(services),
        admin,
        Path(room_id.clone()),
        bytes(&body),
    ))
    .await?;

    Ok(task_started(
        &task,
        &format!("Quarantining the media of {room_id}"),
    ))
}

pub(super) async fn quarantine_user(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: QuarantineUserArgs,
) -> Result<CommandOutput> {
    let user_id = user_id(services, &args.user)?;
    let body = json!({ "reason": args.reason });
    let answer = call(moderation::quarantine_user_media(
        state(services),
        admin,
        Path(user_id.clone()),
        bytes(&body),
    ))
    .await?;

    Ok(done(&format!(
        "Quarantined {} of the {} files {user_id} uploaded.",
        scalar(&answer["quarantined"]),
        scalar(&answer["uploaded"]),
    )))
}

pub(super) async fn purge_media(
    services: &Arc<Services>,
    admin: AdminAuth,
    origin: &str,
    args: PurgeMediaArgs,
) -> Result<CommandOutput> {
    let body = json!({ "origin": origin, "older_than_days": args.days });
    let task = call(moderation::purge_media(
        state(services),
        admin,
        bytes(&body),
    ))
    .await?;

    Ok(task_started(
        &task,
        &format!("Purging {origin} media older than {} days", args.days),
    ))
}

pub(super) async fn purge_history(
    services: &Arc<Services>,
    admin: AdminAuth,
    args: PurgeHistoryArgs,
) -> Result<CommandOutput> {
    let room_id = room_id(services, &args.room).await?;
    let before_ts = match (args.days, args.before_ts) {
        (Some(days), _) => {
            let age = days.saturating_mul(86_400_000);
            Some(now_millis().saturating_sub(age))
        }
        (None, ts) => ts,
    };

    let body = json!({
        "before_ts_ms": before_ts,
        "before_event_id": args.before_event,
        "include_local": args.include_local,
    });

    let task = call(moderation::purge_history(
        state(services),
        admin,
        Path(room_id.clone()),
        bytes(&body),
    ))
    .await?;

    Ok(task_started(
        &task,
        &format!("Purging the history of {room_id}"),
    ))
}

fn mxc_path(mxc: &OwnedMxcUri) -> Result<Path<(String, String)>> {
    let (server_name, media_id) = mxc
        .parts()
        .map_err(|e| err!(Request(InvalidParam("{mxc} is not an mxc:// URI: {e}"))))?;

    if media_id.is_empty() {
        return Err!(Request(InvalidParam("{mxc} names no media.")));
    }

    Ok(Path((server_name.to_string(), media_id.to_owned())))
}

fn task_started(task: &Value, doing: &str) -> CommandOutput {
    let task_id = task["task_id"].as_str().unwrap_or("");

    done(&format!(
        "{doing} as task {task_id}; `!admin server tasks` shows how it went."
    ))
}

fn list(value: &Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .map(scalar)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ruma::user_id;

    use super::super::{Cli, Command, FederationCommand, MediaCommand, RoomCommand, words};

    fn parse(line: &str) -> Result<Cli, clap::Error> {
        let words = words::split(line).expect("balanced quotes");
        Cli::try_parse_from(words::argv(words, user_id!("@phantom:example.com")))
    }

    #[test]
    fn block_takes_a_server_or_a_pattern() {
        let cli = parse("!admin federation block evil.example --reason spam").unwrap();
        assert!(matches!(
            cli.command,
            Command::Federation(FederationCommand::Block(ref args))
                if args.target == "evil.example" && !args.pattern
                    && args.reason.as_deref() == Some("spam")
        ));

        let cli = parse(r"!admin federation unblock '\.evil\.example$' --pattern").unwrap();
        assert!(matches!(
            cli.command,
            Command::Federation(FederationCommand::Unblock(ref args)) if args.pattern
        ));
    }

    #[test]
    fn purge_history_needs_exactly_one_cutoff() {
        assert!(parse("!admin rooms purge-history !r:example.com").is_err());
        assert!(parse("!admin rooms purge-history !r:example.com --days 3 --before-ts 5").is_err());

        let cli =
            parse("!admin rooms purge-history !r:example.com --days 30 --include-local").unwrap();
        assert!(matches!(
            cli.command,
            Command::Rooms(RoomCommand::PurgeHistory(ref args))
                if args.days == Some(30) && args.include_local
        ));
    }

    #[test]
    fn media_moderation_parses() {
        let cli = parse("!admin media quarantine mxc://example.com/abc").unwrap();
        assert!(matches!(
            cli.command,
            Command::Media(MediaCommand::Quarantine(_))
        ));

        let cli = parse("!admin media purge-remote --days 90").unwrap();
        assert!(matches!(
            cli.command,
            Command::Media(MediaCommand::PurgeRemote(ref args)) if args.days == 90
        ));

        assert!(parse("!admin media purge-local").is_err());
    }
}
