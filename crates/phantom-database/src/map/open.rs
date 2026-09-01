//! Taking a lasting handle on a column.

use std::sync::Arc;

use rocksdb::ColumnFamily;

use crate::Engine;

/// The engine's handle for `name`, with its lifetime parameter erased.
///
/// # Panics
///
/// If no column of that name was described before the database was opened.
/// Columns come from a static table, so a miss is a programming error.
#[allow(unsafe_code)]
pub(super) fn open(db: &Arc<Engine>, name: &str) -> Arc<ColumnFamily> {
    let bounded = db.cf(name);
    let bounded_ptr = Arc::into_raw(bounded);
    let cf_ptr = bounded_ptr.cast::<ColumnFamily>();

    unsafe { Arc::from_raw(cf_ptr) }
}
