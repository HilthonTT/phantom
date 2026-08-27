use std::sync::{Arc, RwLock};

use phantom_core::{Result, utils};
use phantom_database::{Database, Map};

pub struct Data {
    global: Arc<Map>,
    counter: RwLock<u64>,
    pub(super) db: Arc<Database>,
}

const COUNTER: &[u8] = b"c";

impl Data {
    pub(super) fn new(args: &crate::Args<'_>) -> Self {
        let db = &args.db;
        Self {
            global: db["global"].clone(),
            counter: RwLock::new(
                Self::stored_count(&db["global"]).expect("initialized global counter"),
            ),
            db: args.db.clone(),
        }
    }

    pub fn next_count(&self) -> Result<u64> {
        let _cork = self.db.db.cork_guard();
        let mut lock = self.counter.write().expect("locked");
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

    #[inline]
    pub fn current_count(&self) -> u64 {
        let lock = self.counter.read().expect("locked");
        let counter: &u64 = &lock;
        debug_assert!(
            *counter == Self::stored_count(&self.global).expect("database failure"),
            "counter mismatch"
        );

        *counter
    }

    #[inline]
    pub fn backup(&self) -> Result {
        self.db.db.backup()
    }

    #[inline]
    pub fn backup_list(&self) -> Result<String> {
        self.db.db.backup_list()
    }

    fn stored_count(global: &Arc<Map>) -> Result<u64> {
        match global.get_blocking(COUNTER) {
            Ok(counter) => utils::bytes::u64_from_bytes(&counter),
            // Nothing has been counted yet on a database that was just
            // created. Only a missing key means that; the reference reads any
            // error as a zero, which would have the next write start the
            // counter over.
            Err(e) if e.is_not_found() => Ok(0),
            Err(e) => Err(e),
        }
    }
}
