//! Online backups.

use std::fmt::Write;

use phantom_core::{Result, error, implement, info, utils::time::rfc2822_from_seconds};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};

use crate::{
    Engine,
    util::{map_err, or_else},
};

/// Takes a backup into `database_backup_path`, then deletes the oldest until
/// no more than `database_backups_to_keep` remain. Does nothing when no backup
/// path is configured.
#[implement(Engine)]
#[tracing::instrument(skip(self))]
pub fn backup(&self) -> Result {
    let server = &self.ctx.server;
    let config = &server.config;
    let Some(path) = backup_path(self) else {
        return Ok(());
    };

    let options = BackupEngineOptions::new(path).map_err(map_err)?;
    let mut engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).map_err(map_err)?;
    if config.database_backups_to_keep > 0 {
        // A read-only or secondary instance cannot flush, and asking it to
        // fails the backup outright.
        let flush = !self.is_read_only();
        engine
            .create_new_backup_flush(&self.db, flush)
            .map_err(map_err)?;

        let engine_info = engine.get_backup_info();
        let info = &engine_info.last().expect("backup engine info is not empty");
        info!(
            "Created database backup #{} using {} bytes in {} files",
            info.backup_id, info.size, info.num_files,
        );
    }

    if config.database_backups_to_keep >= 0 {
        let keep = u32::try_from(config.database_backups_to_keep)?;
        if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
            error!("Failed to purge old backup: {e:?}");
        }
    }

    Ok(())
}

/// The backups present in `database_backup_path`, as a human-readable listing.
#[implement(Engine)]
pub fn backup_list(&self) -> Result<String> {
    let Some(path) = backup_path(self) else {
        return Ok(
            "Configure database_backup_path to enable backups, or the path specified is \
                   not valid"
                .to_owned(),
        );
    };

    let mut res = String::new();
    let options = BackupEngineOptions::new(path).or_else(or_else)?;
    let engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).or_else(or_else)?;
    for info in engine.get_backup_info() {
        writeln!(
            res,
            "#{} {}: {} bytes, {} files",
            info.backup_id,
            rfc2822_from_seconds(info.timestamp),
            info.size,
            info.num_files,
        )?;
    }

    Ok(res)
}

/// The configured backup path, if one is set to anything but the empty string.
fn backup_path(engine: &Engine) -> Option<&std::path::Path> {
    engine
        .ctx
        .server
        .config
        .database_backup_path
        .as_deref()
        .filter(|path| !path.as_os_str().is_empty())
}
