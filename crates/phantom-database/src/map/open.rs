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

    // SAFETY: `BoundColumnFamily<'a>` and `ColumnFamily` are the same handle;
    // the lifetime on the former is how `rocksdb` states that a column handle
    // is invalidated when the database closes or the column is dropped. That
    // borrow cannot be held in a field without infecting `Map` — and through
    // it every service above — with a lifetime parameter, so it is erased
    // here and upheld structurally instead: this handle sits beside the
    // `Arc<Engine>` that keeps the database open, in the same `Map`, so it
    // cannot outlive what it borrows from. Dropping the column while the
    // database is open would also invalidate it, and phantom never does:
    // columns it no longer describes are opened against a tombstone
    // descriptor rather than dropped.
    unsafe { Arc::from_raw(cf_ptr) }
}
