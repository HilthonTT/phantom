use phantom_core::{Result, implement};
use rocksdb::LiveFile as SstFile;

use super::Engine;
use crate::engine::error::result;

#[implement(Engine)]
pub fn file_list(&self) -> Result<impl Iterator<Item = SstFile> + Send + use<>> {
    result(self.db.live_files()).map(Vec::into_iter)
}
