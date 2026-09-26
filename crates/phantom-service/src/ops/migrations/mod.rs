//! One-time database migrations.
//!
//! A fresh database is stamped current and an existing one is walked through
//! the pending steps, once the version and server name gates decide it is safe
//! to touch. This is a plain module run by `Services::start` before any worker
//! starts, not a registered service.

#[cfg(test)]
mod tests;

use std::{cmp::Ordering, time::Duration};

use futures::{StreamExt, TryStreamExt, future::BoxFuture};
use phantom_core::{
    Err, Result, debug_info, info,
    stream::{ReadyExt, TryReadyExt},
    warn,
};
use phantom_database::Deserialized;
use tokio::time::sleep;

use crate::Services;

/// The current schema version.
/// - A database opened at a greater version is refused; the software must be
///   updated for backward-incompatible changes.
/// - A database opened at a lesser version is walked through the pending
///   steps and stamped with this version. Version 0 is a database written
///   before phantom stamped a version at all.
pub(crate) const DATABASE_VERSION: u64 = 1;

const SERVER_NAME_KEY: &[u8] = b"server_name";

const FORCE_MIGRATION_DELAY: Duration = Duration::from_secs(15);

/// A named migration step. The pass reports whether it finished; one that waits
/// on a condition outside the database reports false, leaves its marker
/// unstamped and runs again on a later start.
struct Step {
    marker: &'static str,
    run: for<'a> fn(&'a Services) -> BoxFuture<'a, Result<bool>>,
}

/// Every migration step, in the order they run.
///
/// A fresh database is stamped with every marker listed here, so a step only
/// ever runs against data written before it existed. To add one, write the
/// pass in a submodule and append it; never reorder or rename an entry, since
/// the marker is what records it done.
const STEPS: &[Step] = &[];

pub(crate) async fn migrations(services: &Services) -> Result {
    let config = &services.server.config.database;

    if config.force_migration {
        warn!(
            delay = ?FORCE_MIGRATION_DELAY,
            "The force_migration option is set. THIS IS NOT INTENDED TO BE USED UNDER ANY \
             NORMAL CIRCUMSTANCES AND YOU MAY BE CORRUPTING YOUR DATABASE BY PROCEEDING. \
             Remove force_migration from the configuration to clear this warning; startup \
             continues after the delay."
        );

        sleep(FORCE_MIGRATION_DELAY).await;
    }

    if !config.database_migrations {
        warn!("Skipping database migrations due to configuration...");
        return Ok(());
    }

    // A stop before any step ran leaves nothing to resume from.
    services.server.check_running()?;

    if services.users.count().await == 0 {
        return fresh(services).await;
    }

    check_database_version(services).await?;
    check_server_name(services).await?;

    migrate(services).await.inspect_err(|error| {
        if error.is_interrupted() {
            warn!(
                "Stopped during database migrations. The steps that completed are recorded; the \
                 rest run on the next start."
            );
        }
    })
}

/// Refuses a database stamped by a newer build than this one, unless
/// force_migration asks to stamp it down deliberately.
async fn check_database_version(services: &Services) -> Result {
    let discovered = services.server_state.database_version().await;

    if discovered > DATABASE_VERSION && !services.server.config.database.force_migration {
        return Err!(Database(
            "Database schema version {discovered} is newer than this build supports \
             ({DATABASE_VERSION}). Upgrade phantom to a build supporting this database."
        ));
    }

    Ok(())
}

/// Matrix resource ownership is based on the server name; changing it requires
/// recreating the database from scratch. The marker is stamped once in
/// [`fresh`]; a database from before the marker is backfilled by probing for
/// any user from the configured server.
async fn check_server_name(services: &Services) -> Result {
    let server_name = &services.server.name;

    let existing = services.db["global"]
        .get(SERVER_NAME_KEY)
        .await
        .deserialized::<String>();

    match existing {
        Err(_) => backfill_server_name(services).await,
        Ok(existing) if existing.eq(server_name) => Ok(()),
        Ok(existing) => Err!(Database(
            "Database belongs to {existing}; configured server name is {server_name}. Cannot \
             reuse."
        )),
    }
}

/// Stamps the marker on a database that predates it, provided any user belongs
/// to the configured server. If none does, the database belongs to a different
/// server and reuse is refused.
async fn backfill_server_name(services: &Services) -> Result {
    let server_name = &services.server.name;

    let has_local_user = services
        .users
        .stream()
        .ready_any(|user_id| services.server_state.user_is_local(user_id))
        .await;

    if !has_local_user {
        return Err!(Database(
            "Database has no users from {server_name}; refusing to reuse with this server_name."
        ));
    }

    services.db["global"].insert(SERVER_NAME_KEY, server_name.as_str())?;
    info!(%server_name, "Stamped server_name marker on existing database");

    Ok(())
}

async fn fresh(services: &Services) -> Result {
    let global = &services.db["global"];

    services
        .server_state
        .bump_database_version(DATABASE_VERSION)?;

    global.insert(SERVER_NAME_KEY, services.server.name.as_str())?;

    for step in STEPS {
        global.insert(step.marker, [])?;
    }

    warn!("Created new RocksDB database with version {DATABASE_VERSION}");

    Ok(())
}

/// Applies every pending step, then stamps the schema version.
async fn migrate(services: &Services) -> Result {
    let discovered = services.server_state.database_version().await;

    for step in STEPS {
        if pending(services, step.marker).await? && (step.run)(services).await? {
            services.db["global"].insert(step.marker, [])?;
        }
    }

    services.server.check_running()?;

    // A newer database was already refused unless force_migration asked for
    // the downgrade, so stamping ours is safe.
    services
        .server_state
        .bump_database_version(DATABASE_VERSION)?;

    match discovered.cmp(&DATABASE_VERSION) {
        Ordering::Less => {
            info!("Database: migrated schema version from {discovered} to {DATABASE_VERSION}.");
        }
        Ordering::Greater => warn!(
            "Database: stamped schema version {DATABASE_VERSION} over a higher discovered \
             version {discovered} (forced downgrade)."
        ),
        Ordering::Equal => {}
    }

    warn_forbidden_names(services).await?;

    info!("Loaded RocksDB database with schema version {DATABASE_VERSION}");

    Ok(())
}

/// Warns about existing names the configuration now forbids.
///
/// The patterns are advisory rather than enforced retroactively, so a match
/// only names the user or the alias in the log. Neither scan runs when its own
/// pattern list is empty or once shutdown begins.
async fn warn_forbidden_names(services: &Services) -> Result {
    let rooms = &services.server.config.rooms;

    services.server.check_running()?;

    if !rooms.forbidden_usernames.is_empty() {
        debug_info!("Scanning for forbidden usernames");

        services
            .users
            .stream()
            .map(|user_id| services.server.check_running().map(|()| user_id.to_owned()))
            .try_filter_map(async |user_id| {
                Ok(services
                    .users
                    .is_active_local(&user_id)
                    .await
                    .then_some(user_id))
            })
            .ready_try_for_each(|user_id| {
                let patterns = matched_patterns(&rooms.forbidden_usernames, user_id.localpart());
                if !patterns.is_empty() {
                    warn!("User {user_id} matches forbidden username patterns: {patterns}");
                }

                Ok(())
            })
            .await?;
    }

    services.server.check_running()?;

    if !rooms.forbidden_alias_names.is_empty() {
        debug_info!("Scanning for forbidden alias names");

        services
            .rooms
            .metadata
            .iter_ids()
            .map(|room_id| services.server.check_running().map(|()| room_id.to_owned()))
            .try_for_each(async |room_id| {
                services
                    .rooms
                    .alias
                    .local_aliases_for_room(&room_id)
                    .map(|room_alias| services.server.check_running().map(|()| room_alias))
                    .ready_try_for_each(|room_alias| {
                        let patterns =
                            matched_patterns(&rooms.forbidden_alias_names, room_alias.alias());
                        if !patterns.is_empty() {
                            warn!(
                                "Room {room_id} with alias {room_alias} matches the following \
                                 forbidden alias name patterns: {patterns}"
                            );
                        }

                        Ok(())
                    })
                    .await
            })
            .await?;
    }

    Ok(())
}

/// The patterns in `set` that match `name`, comma separated; empty when none
/// does.
fn matched_patterns(set: &regex::RegexSet, name: &str) -> String {
    set.matches(name)
        .iter()
        .map(|index| set.patterns()[index].as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a named migration step still needs to run, refusing once shutdown
/// begins.
///
/// A step gate is the safe place to observe a stop request: every step that has
/// already run stamped its marker, so the ladder is consistent here and the
/// remaining steps resume on the next start. A step about to run is logged, so
/// the operator sees which one a long boot is spending its time on.
async fn pending(services: &Services, marker: &'static str) -> Result<bool> {
    services.server.check_running()?;

    let pending = !marker_present(services, marker).await?;

    if pending {
        info!(%marker, "Running database migration");
    }

    Ok(pending)
}

/// Whether a migration step has stamped its marker.
///
/// Only a missing marker reads as absent. A read that fails propagates, so a
/// step is never skipped on the strength of a failed read.
async fn marker_present(services: &Services, marker: &str) -> Result<bool> {
    match services.db["global"].get(marker).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => {
            warn!(%marker, %error, "Migration marker failed to read");
            Err(error)
        }
    }
}
