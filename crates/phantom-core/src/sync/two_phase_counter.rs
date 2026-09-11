use std::{
    collections::VecDeque,
    ops::{Deref, Range},
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{Result, checked, error, is_equal_to};

pub trait Commit: Fn(u64) -> Result + Send + Sync {}

impl<F> Commit for F where F: Fn(u64) -> Result + Send + Sync {}

pub struct Counter<C: Commit, R: Commit> {
    inner: RwLock<State<C, R>>,
}

struct State<C: Commit, R: Commit> {
    dispatched: u64,

    commit: C,

    pending: VecDeque<u64>,

    release: R,
}

#[clippy::has_significant_drop]
pub struct Permit<C: Commit, R: Commit> {
    counter: Arc<Counter<C, R>>,

    retired: u64,

    id: u64,
}

impl<C: Commit, R: Commit> Counter<C, R> {
    pub fn new(init: u64, commit: C, release: R) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(State {
                dispatched: init,
                commit,
                pending: VecDeque::new(),
                release,
            }),
        })
    }

    pub fn next(self: &Arc<Self>) -> Result<Permit<C, R>> {
        let (retired, id) = self.write().dispatch()?;

        Ok(Permit {
            counter: self.clone(),
            retired,
            id,
        })
    }

    #[inline]
    pub fn current(&self) -> u64 {
        self.read().retired()
    }

    #[inline]
    pub fn dispatched(&self) -> u64 {
        self.read().dispatched
    }

    #[inline]
    pub fn range(&self) -> Range<u64> {
        let state = self.read();

        state.retired()..state.dispatched
    }

    #[inline]
    fn read(&self) -> RwLockReadGuard<'_, State<C, R>> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    #[inline]
    fn write(&self) -> RwLockWriteGuard<'_, State<C, R>> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<C: Commit, R: Commit> State<C, R> {
    fn dispatch(&mut self) -> Result<(u64, u64)> {
        let retired = self.retired();
        let prev = self.dispatched;
        let dispatched = checked!(prev + 1)?;

        debug_assert!(
            !self.is_pending(dispatched),
            "a sequence number cannot already be pending"
        );

        (self.commit)(dispatched)?;
        self.pending.push_back(dispatched);
        self.dispatched = dispatched;

        Ok((retired, dispatched))
    }

    fn retire(&mut self, id: u64) {
        debug_assert!(
            self.is_pending(id),
            "a sequence number must be pending to retire"
        );

        let Some(index) = self.pending_index(id) else {
            error!(id, "Sequence number was not pending for retirement.");
            return;
        };

        let removed = self.pending.remove(index);
        debug_assert_eq!(
            removed,
            Some(id),
            "the number removed must be the one given"
        );

        if index != 0 {
            return;
        }

        let release = if self.pending.is_empty() {
            self.dispatched
        } else {
            id
        };

        debug_assert!(release >= id, "the frontier must not move backwards");

        (self.release)(release)
            .inspect_err(|error| error!(release, %error, "Failed to release sequence number."))
            .ok();
    }

    fn retired(&self) -> u64 {
        debug_assert!(
            self.pending.iter().is_sorted(),
            "pending numbers are pushed in order and so are always sorted"
        );

        self.pending
            .front()
            .map_or(self.dispatched, |pending| pending.saturating_sub(1))
    }

    fn pending_index(&self, id: u64) -> Option<usize> {
        debug_assert!(
            self.pending.iter().is_sorted(),
            "pending numbers are pushed in order and so are always sorted"
        );

        self.pending.binary_search(&id).ok()
    }

    fn is_pending(&self, id: u64) -> bool {
        self.pending.iter().any(is_equal_to!(&id))
    }
}

impl<C: Commit, R: Commit> Permit<C, R> {
    #[inline]
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    #[inline]
    #[must_use]
    pub fn retired(&self) -> u64 {
        self.retired
    }
}

impl<C: Commit, R: Commit> Deref for Permit<C, R> {
    type Target = u64;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl<C: Commit, R: Commit> Drop for Permit<C, R> {
    fn drop(&mut self) {
        self.counter.write().retire(self.id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    type Log = Arc<Mutex<Vec<u64>>>;

    fn counter(init: u64) -> (Arc<Counter<impl Commit, impl Commit>>, Log, Log) {
        let committed = Arc::new(Mutex::new(Vec::new()));
        let released = Arc::new(Mutex::new(Vec::new()));

        let c = committed.clone();
        let r = released.clone();

        let counter = Counter::new(
            init,
            move |id| {
                c.lock().expect("locked").push(id);
                Ok(())
            },
            move |id| {
                r.lock().expect("locked").push(id);
                Ok(())
            },
        );

        (counter, committed, released)
    }

    #[test]
    fn the_first_number_follows_the_initial_value() {
        let (counter, committed, _) = counter(41);

        let permit = counter.next().expect("drawn");

        assert_eq!(*permit, 42);
        assert_eq!(permit.id(), 42);
        assert_eq!(*committed.lock().expect("locked"), vec![42]);
    }

    #[test]
    fn a_pending_number_is_dispatched_but_not_retired() {
        let (counter, _, _) = counter(0);

        let permit = counter.next().expect("drawn");

        assert_eq!(counter.dispatched(), 1, "handed out");
        assert_eq!(counter.current(), 0, "not yet visible");
        assert_eq!(counter.range(), 0..1);

        drop(permit);

        assert_eq!(counter.current(), 1, "visible once the write is done");
        assert_eq!(counter.range(), 1..1);
    }

    #[test]
    fn the_frontier_waits_for_the_oldest_write() {
        let (counter, _, released) = counter(0);

        let first = counter.next().expect("drawn");
        let second = counter.next().expect("drawn");
        let third = counter.next().expect("drawn");

        drop(third);
        assert_eq!(counter.current(), 0, "1 and 2 are still in flight");
        assert!(released.lock().expect("locked").is_empty());

        drop(second);
        assert_eq!(counter.current(), 0, "1 is still in flight");

        drop(first);
        assert_eq!(counter.current(), 3, "everything is now visible");
        assert_eq!(*released.lock().expect("locked"), vec![3]);
    }

    #[test]
    fn the_frontier_stops_below_what_is_still_pending() {
        let (counter, _, released) = counter(0);

        let first = counter.next().expect("drawn");
        let second = counter.next().expect("drawn");

        drop(first);
        assert_eq!(counter.current(), 1, "2 is still in flight");
        assert_eq!(*released.lock().expect("locked"), vec![1]);

        drop(second);
        assert_eq!(counter.current(), 2);
    }

    #[test]
    fn a_permit_samples_the_frontier_it_was_drawn_at() {
        let (counter, _, _) = counter(10);

        let first = counter.next().expect("drawn");
        assert_eq!(first.retired(), 10);

        let second = counter.next().expect("drawn");
        assert_eq!(second.retired(), 10, "1 is still in flight");

        drop(first);
        drop(second);

        let third = counter.next().expect("drawn");
        assert_eq!(third.retired(), 12);
    }

    #[test]
    fn a_failed_commit_draws_nothing() {
        let counter = Counter::new(7, |_| Err(crate::err!("no")), |_| Ok(()));

        assert!(counter.next().is_err());
        assert_eq!(counter.dispatched(), 7);
        assert_eq!(counter.current(), 7);
    }
}
