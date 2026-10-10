use std::{
    fmt::Write,
    sync::{Mutex, TryLockError},
};

use phantom_core::{Err, Result, error, implement, info, time::rfc2822_from_seconds};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};

use crate::{
    Engine,
    engine::error::{map_err, or_else},
};

#[implement(Engine)]
#[tracing::instrument(skip(self))]
pub fn backup(&self) -> Result {
    let server = &self.ctx.server;
    let config = &server.config;
    let Some(path) = backup_path(self) else {
        return Ok(());
    };

    // Two backup engines writing one directory at once can trash it: each
    // collects the files the other has not recorded yet as garbage.
    static RUNNING: Mutex<()> = Mutex::new(());
    let _running = match RUNNING.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => {
            return Err!(Conflict("A database backup is already running."));
        }
    };

    let options = BackupEngineOptions::new(path).map_err(map_err)?;
    let mut engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).map_err(map_err)?;
    if config.database.database_backups_to_keep > 0 {
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

    if config.database.database_backups_to_keep >= 0 {
        let keep = u32::try_from(config.database.database_backups_to_keep)?;
        if let Err(e) = engine.purge_old_backups(keep.try_into()?) {
            error!("Failed to purge old backup: {e:?}");
        }
    }

    Ok(())
}

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

/// The newest backup's time, in seconds since the epoch, and size in bytes;
/// none when backups are off or none was made yet.
#[implement(Engine)]
pub fn last_backup(&self) -> Result<Option<(i64, u64)>> {
    // Opening a backup engine creates its directories; only reading, leave
    // a directory no backup made yet alone.
    let Some(path) = backup_path(self).filter(|path| path.is_dir()) else {
        return Ok(None);
    };

    let options = BackupEngineOptions::new(path).or_else(or_else)?;
    let engine = BackupEngine::open(&options, &*self.ctx.env.lock()?).or_else(or_else)?;

    Ok(engine
        .get_backup_info()
        .into_iter()
        .max_by_key(|info| info.timestamp)
        .map(|info| (info.timestamp, info.size)))
}

fn backup_path(engine: &Engine) -> Option<&std::path::Path> {
    engine
        .ctx
        .server
        .config
        .database
        .database_backup_path
        .as_deref()
        .filter(|path| !path.as_os_str().is_empty())
}
