use std::sync::{
    Arc, RwLock,
    atomic::{AtomicUsize, Ordering},
};

use super::Capture;

pub struct State {
    pub(super) active: RwLock<Vec<Arc<Capture>>>,

    count: AtomicUsize,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: RwLock::new(Vec::new()),
            count: AtomicUsize::new(0),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.count.load(Ordering::Relaxed) > 0
    }

    pub(super) fn add(&self, capture: &Arc<Capture>) {
        let mut active = self.active.write().expect("locked for writing");
        active.push(capture.clone());
        self.count.store(active.len(), Ordering::Relaxed);
    }

    pub(super) fn del(&self, capture: &Arc<Capture>) {
        let mut active = self.active.write().expect("locked for writing");
        if let Some(pos) = active.iter().position(|item| Arc::ptr_eq(item, capture)) {
            active.swap_remove(pos);
        }

        self.count.store(active.len(), Ordering::Relaxed);
    }
}
