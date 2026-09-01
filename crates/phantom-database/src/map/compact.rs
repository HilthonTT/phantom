//! Compacting a column on demand.
//!
//! The engine compacts on its own as writes accumulate. This is for the times
//! an operator wants it done now and to a shape of their choosing — after a
//! bulk delete, say, where the space is not reclaimed until the tombstones are
//! compacted away.

use phantom_core::{Err, Result, implement};
use rocksdb::{BottommostLevelCompaction, CompactOptions};

use crate::keyval::KeyBuf;

/// What to compact, and how thoroughly.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The key range to compact, as `(start, end)`. `None` at either end
    /// means the column's own bound.
    pub range: (Option<KeyBuf>, Option<KeyBuf>),

    /// Which levels to compact, as `(from, into)`:
    ///
    /// - `(None, None)` — every level, into wherever each belongs.
    /// - `(None, Some(n))` — every level, into level `n`.
    /// - `(Some(n), None)` — level `n`, back into level `n`.
    ///
    /// Naming both is rejected: the engine picks the source levels itself
    /// once a target is given.
    pub level: (Option<usize>, Option<usize>),

    /// Keep compacting until nothing is left to do. Otherwise one pass runs
    /// and whatever it produces is left alone.
    pub exhaustive: bool,

    /// Wait for the compactions already running to finish, run this one with
    /// the column to itself, and only then let automatic compaction resume.
    pub exclusive: bool,
}

/// Compacts the column, blocking until it is done.
///
/// Compaction is measured in minutes on a large column and holds the storage
/// throughout, which is why there is no non-blocking form: this belongs on a
/// thread of its own, driven by an operator command.
#[implement(super::Map)]
#[tracing::instrument(name = "compact", level = "info", skip(self), fields(%self))]
pub fn compact_blocking(&self, opts: Options) -> Result {
    let mut co = CompactOptions::default();
    co.set_exclusive_manual_compaction(opts.exclusive);
    co.set_bottommost_level_compaction(if opts.exhaustive {
        BottommostLevelCompaction::Force
    } else {
        BottommostLevelCompaction::ForceOptimized
    });

    match opts.level {
        (None, None) => {
            co.set_change_level(true);
            co.set_target_level(-1);
        }
        (None, Some(level)) => {
            co.set_change_level(true);
            co.set_target_level(level.try_into()?);
        }
        (Some(level), None) => {
            co.set_change_level(false);
            co.set_target_level(level.try_into()?);
        }
        (Some(_), Some(_)) => return Err!("compacting between two named levels is not supported"),
    }

    self.db
        .db
        .compact_range_cf_opt(&self.cf(), opts.range.0, opts.range.1, &co);

    Ok(())
}
