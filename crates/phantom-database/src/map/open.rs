use std::sync::Arc;

use rocksdb::ColumnFamily;

use crate::Engine;

#[allow(unsafe_code)]
pub(super) fn open(db: &Arc<Engine>, name: &str) -> Arc<ColumnFamily> {
    let bounded = db.cf(name);
    let bounded_ptr = Arc::into_raw(bounded);
    let cf_ptr = bounded_ptr.cast::<ColumnFamily>();

    unsafe { Arc::from_raw(cf_ptr) }
}
