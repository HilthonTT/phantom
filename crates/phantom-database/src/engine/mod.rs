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

    pub(crate) pool: Arc<Pool>,

    pub(crate) ctx: Arc<Context>,

    columns: Vec<String>,

    read_only: bool,
    secondary: bool,
    checksums: bool,
    corks: AtomicU32,
}

pub(crate) type Db = DBWithThreadMode<MultiThreaded>;

impl Engine {
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

    #[tracing::instrument(
        level = "info",
        skip_all,
        fields(
            sequence = ?self.current_sequence(),
        ),
    )]
    pub fn sort(&self) -> Result {
        let flushoptions = FlushOptions::default();

        let cfs: Vec<_> = self.columns.iter().map(|name| self.cf(name)).collect();
        let cfs: Vec<&_> = cfs.iter().collect();

        result(self.db.flush_cfs_opt(&cfs, &flushoptions))
    }

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

    #[tracing::instrument(level = "info", skip_all)]
    pub fn sync(&self) -> Result {
        result(DBCommon::flush_wal(&self.db, true))
    }

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

    #[inline]
    #[must_use]
    pub fn corked(&self) -> bool {
        self.corks.load(Ordering::Relaxed) > 0
    }

    pub fn property_integer(&self, cf: &impl AsColumnFamilyRef, name: &CStr) -> Result<u64> {
        result(self.db.property_int_value_cf(cf, name))
            .and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
    }

    pub fn property(&self, cf: &impl AsColumnFamilyRef, name: &str) -> Result<String> {
        result(self.db.property_value_cf(cf, name))
            .and_then(|val| val.map_or_else(|| Err!("Property {name:?} not found."), Ok))
    }

    #[must_use]
    pub fn cf(&self, name: &str) -> Arc<BoundColumnFamily<'_>> {
        self.db
            .cf_handle(name)
            .expect("column must be described prior to database open")
    }

    #[inline]
    #[must_use]
    #[tracing::instrument(name = "sequence", level = "debug", skip_all, fields(sequence))]
    pub fn current_sequence(&self) -> u64 {
        let sequence = self.db.latest_sequence_number();

        #[cfg(debug_assertions)]
        tracing::Span::current().record("sequence", sequence);

        sequence
    }

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
