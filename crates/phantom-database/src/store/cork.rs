use std::sync::Arc;

use crate::Engine;

pub struct Cork {
    db: Arc<Engine>,
    flush: bool,
    sync: bool,
}

impl Engine {
    #[inline]
    #[must_use]
    pub fn cork_guard(self: &Arc<Self>) -> Cork {
        Cork::new(self, false, false)
    }

    #[inline]
    #[must_use]
    pub fn cork_and_flush(self: &Arc<Self>) -> Cork {
        Cork::new(self, true, false)
    }

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
