use phantom_core::{Result, implement};
use rocksdb::{perf::MemoryUsageBuilder, properties};

use super::Engine;
use crate::engine::error::{or_else, result};

/// The engine's figures for a metrics scrape, all of them read from what
/// RocksDB already tracks, so taking them costs no I/O.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Bytes held by each block cache: `row`, then one per column cache.
    pub cache_usage: Vec<(String, u64)>,

    /// The memtables' size, and the part of it not yet flushed.
    pub mem_tables: u64,
    pub mem_tables_unflushed: u64,

    /// Memory the open table readers hold, indexes and filters included.
    pub table_readers: u64,

    /// Each column's SST files on disk, obsolete ones still referenced
    /// included.
    pub sst_bytes: Vec<(String, u64)>,

    /// The bytes compaction estimates it still has to rewrite, summed over
    /// the columns.
    pub pending_compaction_bytes: u64,

    pub running_compactions: u64,
}

#[implement(Engine)]
pub fn stats(&self) -> Result<Stats> {
    let mut stats = Stats::default();

    {
        let row_cache = self.ctx.row_cache.lock()?;
        let col_cache = self.ctx.col_cache.lock()?;

        let mut builder = MemoryUsageBuilder::new().or_else(or_else)?;
        builder.add_db(&self.db);
        builder.add_cache(&row_cache);
        let usage = builder.build().or_else(or_else)?;

        stats.mem_tables = usage.approximate_mem_table_total();
        stats.mem_tables_unflushed = usage.approximate_mem_table_unflushed();
        stats.table_readers = usage.approximate_mem_table_readers_total();

        stats
            .cache_usage
            .push(("row".to_owned(), u64::try_from(row_cache.get_usage())?));
        for (name, cache) in &*col_cache {
            stats
                .cache_usage
                .push((name.clone(), u64::try_from(cache.get_usage())?));
        }
    }

    // A property RocksDB does not keep for a column reads as zero rather than
    // failing the scrape.
    let int = |cf: &_, name| {
        result(self.db.property_int_value_cf(cf, name)).map(Option::unwrap_or_default)
    };

    for name in &self.columns {
        let cf = self.cf(name);

        stats
            .sst_bytes
            .push((name.clone(), int(&cf, properties::TOTAL_SST_FILES_SIZE)?));
        stats.pending_compaction_bytes = stats
            .pending_compaction_bytes
            .saturating_add(int(&cf, properties::ESTIMATE_PENDING_COMPACTION_BYTES)?);
    }

    stats.running_compactions = result(
        self.db
            .property_int_value(properties::NUM_RUNNING_COMPACTIONS),
    )
    .map(Option::unwrap_or_default)?;

    Ok(stats)
}
