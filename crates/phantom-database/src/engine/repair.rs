use std::path::Path;

use phantom_core::{Err, Result, info, warn};
use rocksdb::Options;

use super::Db;

pub(super) fn repair(db_opts: &Options, path: &Path) -> Result {
    warn!("Starting database repair. This may take a long time...");
    match Db::repair(db_opts, path) {
        Ok(()) => info!("Database repair successful."),
        Err(e) => return Err!("Repair failed: {e:?}"),
    }

    Ok(())
}
