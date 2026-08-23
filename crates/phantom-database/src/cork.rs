//! Batching the write-ahead log flush across a burst of writes.

use std::sync::Arc;

use crate::Engine;

/// While a cork is held, writers skip the flush they would otherwise do after
/// each write; the flush happens once, when the cork is dropped. Corks nest,
/// and only the last one to drop flushes.
pub struct Cork {
    db: Arc<Engine>,
    flush: bool,
    sync: bool,
}

impl Engine {
    /// Holds back the flushes for the lifetime of the returned cork.
    #[inline]
    #[must_use]
    pub fn cork_guard(self: &Arc<Self>) -> Cork {
        Cork::new(self, false, false)
    }

    /// As [`Self::cork_guard`], flushing the write-ahead log when the cork
    /// drops.
    #[inline]
    #[must_use]
    pub fn cork_and_flush(self: &Arc<Self>) -> Cork {
        Cork::new(self, true, false)
    }

    /// As [`Self::cork_and_flush`], additionally waiting for the storage to
    /// acknowledge the write.
    #[inline]
    #[must_use]
    pub fn cork_and_sync(self: &Arc<Self>) -> Cork {
        Cork::new(self, true, true)
    }
}

impl Cork {
    #[inline]
    fn new(db: &Arc<Engine>, flush: bool, sync: bool) -> Self {
        db.cork();

        Self {
            db: db.clone(),
            flush,
            sync,
        }
    }
}

impl Drop for Cork {
    fn drop(&mut self) {
        self.db.uncork();
        if self.flush {
            self.db.flush().ok();
        }
        if self.sync {
            self.db.sync().ok();
        }
    }
}
