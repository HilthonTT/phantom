//! The server's monotonic event counter.
//!
//! Every PDU and every account-data change is ordered by a number drawn from
//! here, so it must never repeat and never go backwards. The live value is
//! held in memory and written through to the `global` column on each draw;
//! the debug assertions compare the two on every access so that a divergence
//! surfaces where it was introduced rather than at the next restart.

use std::sync::{Arc, RwLock};

use phantom_core::{Result, bytes};
use phantom_database::{Engine, Map};

/// The key the count is stored under in the `global` column.
const COUNTER: &[u8] = b"c";

pub struct Counter {
    global: Arc<Map>,
    count: RwLock<u64>,
    engine: Arc<Engine>,
}

impl Counter {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let global = args.db["global"].clone();

        Self {
            count: RwLock::new(Self::stored_count(&global).expect("initialized global counter")),
            global,
            engine: args.db.engine.clone(),
        }
    }

    /// The next number in the sequence, written through before it is handed
    /// out so that a crash cannot reissue it.
    pub fn next(&self) -> Result<u64> {
        let _cork = self.engine.cork_guard();
        let mut lock = self.count.write().expect("locked");
        let counter: &mut u64 = &mut lock;
        debug_assert!(
            *counter == Self::stored_count(&self.global).expect("database failure"),
            "counter mismatch"
        );

        *counter = counter
            .checked_add(1)
            .expect("counter must not overflow u64");

        self.global.insert(COUNTER, counter.to_be_bytes())?;

        Ok(*counter)
    }

    /// The last number handed out.
    #[inline]
    pub fn current(&self) -> u64 {
        let lock = self.count.read().expect("locked");
        let counter: &u64 = &lock;
        debug_assert!(
            *counter == Self::stored_count(&self.global).expect("database failure"),
            "counter mismatch"
        );

        *counter
    }

    fn stored_count(global: &Arc<Map>) -> Result<u64> {
        match global.get_blocking(COUNTER) {
            Ok(counter) => bytes::u64_from_bytes(&counter),
            // Nothing has been counted yet on a database that was just
            // created. Only a missing key means that; the reference reads any
            // error as a zero, which would have the next write start the
            // counter over.
            Err(e) if e.is_not_found() => Ok(0),
            Err(e) => Err(e),
        }
    }
}
