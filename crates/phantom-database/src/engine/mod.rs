//! The database instance phantom's columns live in.
//!
//! [`Engine`] owns the open database handle and the operations that act on it
//! as a whole — flushing, compaction, backup, and the properties an operator
//! queries it for. The typed per-column surface is layered on top of this.

mod backup;
mod column_options;
pub mod context;
mod database_options;
pub mod descriptor;
pub(crate) mod error;
mod files;
mod memory_usage;
mod open;
mod repair;

use std::{
    ffi::CStr,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use phantom_core::{Err, Result, debug, info};
use rocksdb::{
    AsColumnFamilyRef, BoundColumnFamily, DBCommon, DBWithThreadMode, FlushOptions, MultiThreaded,
    WaitForCompactOptions,
};

pub use self::context::Context;
use crate::{engine::error::result, pool::Pool};

pub struct Engine {
    pub(crate) db: Db,

    /// The threads blocking reads are offloaded to. Owned by the engine so
    /// that it outlives them: a worker holds column handles into this
    /// database and must be joined before it closes.
    pub(crate) pool: Arc<Pool>,

    pub(crate) ctx: Arc<Context>,
    read_only: bool,
    secondary: bool,
    checksums: bool,
    corks: AtomicU32,
}

pub(crate) type Db = DBWithThreadMode<MultiThreaded>;

impl Engine {
    /// Blocks until every pending compaction has finished.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            sequence = ?self.current_sequence(),
        ),
    )]
    pub fn wait_compactions_blocking(&self) -> Result {
        let mut opts = WaitForCompactOptions::default();
        opts.set_abort_on_pause(true);
        opts.set_flush(false);
        opts.set_timeout(0);

        result(self.db.wait_for_compact(&opts))
    }

    /// Flushes every column's memtable to disk.
    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            sequence = ?self.current_sequence(),
        ),
    )]
    pub fn sort(&self) -> Result {
        let flushoptions = FlushOptions::default();

        result(DBCommon::flush_opt(&self.db, &flushoptions))
    }

    /// Catches a secondary instance up with the primary's writes.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            sequence = ?self.current_sequence(),
        ),
    )]
    pub fn update(&self) -> Result {
        result(self.db.try_catch_up_with_primary())
    }

    /// Flushes the write-ahead log and waits for the storage to acknowledge it.
    #[tracing::instrument(level = "info", skip_all)]
    pub fn sync(&self) -> Result {
        result(DBCommon::flush_wal(&self.db, true))
    }

    /// Flushes the write-ahead log without waiting for durability.
    ///
    /// Writes are buffered until this is called: the engine is opened with
    /// manual WAL flush so that a burst of writes costs one flush rather than
    /// one per write.
    #[tracing::instrument(level = "debug", skip_all)]
    pub fn flush(&self) -> Result {
        result(DBCommon::flush_wal(&self.db, false))
    }

    #[inline]
    pub(crate) fn cork(&self) {
        self.corks.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn uncork(&self) {
        self.corks.fetch_sub(1, Ordering::Relaxed);
    }

    /// Whether a [`Cork`](crate::Cork) is currently held, which is the signal
    /// to writers that they should not flush after every write.
    #[inline]
    #[must_use]
    pub fn corked(&self) -> bool {
        self.corks.load(Ordering::Relaxed) > 0
    }

    /// Queries a database property by null-terminated name, for the properties
    /// that have an integer representation. Intended for low-overhead
    /// programmatic use; see [`Self::property`] for the rest.
    pub fn property_integer(&self, cf: &impl AsColumnFamilyRef, name: &CStr) -> Result<u64> {
        result(self.db.property_int_value_cf(cf, name))
            .and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
    }

    /// Queries a database property by name, receiving the result as a string.
    pub fn property(&self, cf: &impl AsColumnFamilyRef, name: &str) -> Result<String> {
        result(self.db.property_value_cf(cf, name))
            .and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
    }

    /// The handle for a column, which must have been described before the
    /// database was opened.
    ///
    /// # Panics
    ///
    /// If no column of that name was described. Columns come from a static
    /// table, so a miss is a programming error rather than a runtime
    /// condition.
    #[must_use]
    pub fn cf(&self, name: &str) -> Arc<BoundColumnFamily<'_>> {
        self.db
            .cf_handle(name)
            .expect("column must be described prior to database open")
    }

    /// The sequence number of the most recent write, which identifies the
    /// point in time the database is currently at.
    #[inline]
    #[must_use]
    #[tracing::instrument(name = "sequence", level = "debug", skip_all, fields(sequence))]
    pub fn current_sequence(&self) -> u64 {
        let sequence = self.db.latest_sequence_number();

        #[cfg(debug_assertions)]
        tracing::Span::current().record("sequence", sequence);

        sequence
    }

    /// Whether reads verify block checksums, which the map layer consults when
    /// building its read options.
    #[inline]
    #[must_use]
    pub fn checksums(&self) -> bool {
        self.checksums
    }

    #[inline]
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.secondary || self.read_only
    }

    #[inline]
    #[must_use]
    pub fn is_secondary(&self) -> bool {
        self.secondary
    }
}

impl Drop for Engine {
    #[cold]
    fn drop(&mut self) {
        const BLOCKING: bool = true;

        // Before anything else: the workers hold column handles into this
        // database, so they have to be gone before it closes.
        debug!("Waiting for database workers to finish...");
        self.pool.close();

        debug!("Waiting for background tasks to finish...");
        self.db.cancel_all_background_work(BLOCKING);

        info!(
            sequence = %self.current_sequence(),
            "Closing database..."
        );
    }
}
