//! What the open database is holding in memory.

use std::fmt::Write;

use phantom_core::{Result, implement};
use rocksdb::perf::MemoryUsageBuilder;

use super::Engine;
use crate::util::or_else;

/// A human-readable breakdown of the engine's memory, for the admin command
/// that reports it.
///
/// Built through [`MemoryUsageBuilder`] rather than `get_memory_usage_stats`,
/// which only accepts a single-threaded database handle.
#[implement(Engine)]
pub fn memory_usage(&self) -> Result<String> {
    let row_cache = self.ctx.row_cache.lock()?;
    let col_cache = self.ctx.col_cache.lock()?;

    let mut builder = MemoryUsageBuilder::new().or_else(or_else)?;
    builder.add_db(&self.db);
    builder.add_cache(&row_cache);
    let stats = builder.build().or_else(or_else)?;

    let mut res = String::new();
    writeln!(
        res,
        "Memory buffers: {:.2} MiB\nPending write: {:.2} MiB\nTable readers: {:.2} MiB\nRow \
         cache: {:.2} MiB",
        mibs(stats.approximate_mem_table_total()),
        mibs(stats.approximate_mem_table_unflushed()),
        mibs(stats.approximate_mem_table_readers_total()),
        mibs(u64::try_from(row_cache.get_usage())?),
    )?;

    for (name, cache) in &*col_cache {
        writeln!(
            res,
            "{name} cache: {:.2} MiB",
            mibs(u64::try_from(cache.get_usage())?)
        )?;
    }

    Ok(res)
}

/// Bytes as mebibytes.
///
/// The reference implementation routes this through `u32`, which silently
/// reports anything past 4 TiB as zero.
#[allow(clippy::as_conversions, clippy::cast_precision_loss)]
fn mibs(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
