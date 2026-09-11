use phantom_core::{Err, Result, implement};
use rocksdb::{BottommostLevelCompaction, CompactOptions};

use crate::keyval::KeyBuf;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub range: (Option<KeyBuf>, Option<KeyBuf>),

    pub level: (Option<usize>, Option<usize>),

    pub exhaustive: bool,

    pub exclusive: bool,
}

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
