//! The SST files backing the open database.

use phantom_core::{Result, implement};
use rocksdb::LiveFile as SstFile;

use super::Engine;
use crate::util::result;

/// Every SST file the database is currently built from, for the admin commands
/// that report on storage.
///
/// The reference implementation folds a failure here into an empty iterator,
/// which reads as "no files" at the callsite; this reports it instead.
#[implement(Engine)]
pub fn file_list(&self) -> Result<impl Iterator<Item = SstFile> + Send + use<>> {
    result(self.db.live_files()).map(Vec::into_iter)
}
