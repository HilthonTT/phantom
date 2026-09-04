//! A counter that distinguishes "a number has been handed out" from "the write
//! it was handed out for is visible".
//!
//! Every PDU and every account-data change is ordered by a number drawn from a
//! counter, and a sync reads forward from the number it last saw. A plain
//! counter cannot serve both: the writer draws `n` and then writes, and in the
//! window between those a reader that samples the counter sees `n`, finds
//! nothing under it, and moves its token past it. The write lands afterwards
//! and is never sent to that client.
//!
//! So the count has two frontiers. `dispatched` is the highest number handed
//! out, which is what a writer needs; `retired` is one below the lowest number
//! still in flight, which is the highest a reader may safely go. A number is
//! held by a [`Permit`], and retires when the permit drops — that is, when the
//! writer's scope ends and its write is done.
//!
//! Numbers retire out of order but the frontier only ever moves in order: a
//! permit that finishes while an older one is outstanding leaves `retired`
//! where it is, because the older write is still not visible.

use std::{
    collections::VecDeque,
    ops::{Deref, Range},
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{Result, checked, error, is_equal_to};

/// What a counter calls to persist a number or to publish the frontier.
///
/// A trait with a blanket impl rather than a bare `Fn` bound repeated at every
/// use, and rather than one type parameter for both callbacks: they are two
/// different closures, so they are two different types.
pub trait Commit: Fn(u64) -> Result + Send + Sync {}

impl<F> Commit for F where F: Fn(u64) -> Result + Send + Sync {}

/// A two-phase counter. Build one with [`Counter::new`].
pub struct Counter<C: Commit, R: Commit> {
    inner: RwLock<State<C, R>>,
}

/// A counter's state, which every operation takes the lock to reach.
struct State<C: Commit, R: Commit> {
    /// The highest number handed out. The next one is this plus one.
    dispatched: u64,

    /// Persists a number as it is drawn, so that a restart does not hand out
    /// a number a previous run already used.
    commit: C,

    /// The numbers handed out whose permits have not dropped, in order. One
    /// below the front of this is the retirement frontier.
    pending: VecDeque<u64>,

    /// Announces the retirement frontier as it advances. Called from a permit's
    /// destructor, so it must not panic.
    release: R,
}

/// One number, held until the write it was drawn for is done.
///
/// Dereferences to the number. Dropping it retires the number, which is what
/// lets the frontier advance — so a permit must live exactly as long as the
/// write, and no longer.
#[clippy::has_significant_drop]
pub struct Permit<C: Commit, R: Commit> {
    counter: Arc<Counter<C, R>>,

    /// The frontier as it stood when this permit was drawn, sampled under the
    /// same lock as a courtesy to a caller that wants both.
    retired: u64,

    id: u64,
}

impl<C: Commit, R: Commit> Counter<C, R> {
    /// A counter whose first number will be `init + 1`.
    ///
    /// `init` is treated as already retired, which is what makes a counter
    /// restored from storage pick up where the last run left off.
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

    /// Draws the next number, held until the returned permit drops.
    pub fn next(self: &Arc<Self>) -> Result<Permit<C, R>> {
        let (retired, id) = self.write().dispatch()?;

        Ok(Permit {
            counter: self.clone(),
            retired,
            id,
        })
    }

    /// The highest number safe to read up to: everything at or below it has
    /// been written and is visible.
    #[inline]
    pub fn current(&self) -> u64 {
        self.read().retired()
    }

    /// The highest number handed out, whether or not its write has landed.
    #[inline]
    pub fn dispatched(&self) -> u64 {
        self.read().dispatched
    }

    /// Both frontiers, read together. The numbers in this range are the ones
    /// drawn but not yet visible.
    #[inline]
    pub fn range(&self) -> Range<u64> {
        let state = self.read();

        state.retired()..state.dispatched
    }

    /// Reads the state, tolerating a poisoned lock.
    ///
    /// Poisoning carries no information here: `commit` runs before any mutation
    /// and `release` after all of them, so an unwind through either leaves the
    /// state consistent. Honouring the flag instead would turn one failed write
    /// into a permanent outage of every sequence number.
    #[inline]
    fn read(&self) -> RwLockReadGuard<'_, State<C, R>> {
        self.inner.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the state. See [`read`](Self::read) on the poisoning.
    #[inline]
    fn write(&self) -> RwLockWriteGuard<'_, State<C, R>> {
        self.inner.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<C: Commit, R: Commit> State<C, R> {
    /// Hands out the next number, returning the frontier alongside it.
    ///
    /// The number is persisted before it is recorded as pending: a crash
    /// between the two loses a number, which is harmless, where the other order
    /// would hand the same number out twice after a restart.
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

    /// Retires `id`, advancing the frontier if it was the oldest outstanding.
    ///
    /// Runs from a destructor, so a desynchronised pending list or a failing
    /// release is logged rather than raised outside debug assertions.
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

        // Retiring anything but the oldest leaves the frontier where it is:
        // the older write is still not visible, so nothing behind it is safe
        // to read yet.
        if index != 0 {
            return;
        }

        // With nothing left pending everything drawn is visible, so the
        // frontier jumps to the whole dispatched value rather than to this id.
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

    /// One below the lowest number still in flight, or everything dispatched
    /// where nothing is.
    fn retired(&self) -> u64 {
        debug_assert!(
            self.pending.iter().is_sorted(),
            "pending numbers are pushed in order and so are always sorted"
        );

        self.pending
            .front()
            .map_or(self.dispatched, |pending| pending.saturating_sub(1))
    }

    /// Where `id` sits in the pending list.
    fn pending_index(&self, id: u64) -> Option<usize> {
        debug_assert!(
            self.pending.iter().is_sorted(),
            "pending numbers are pushed in order and so are always sorted"
        );

        self.pending.binary_search(&id).ok()
    }

    /// A linear scan for `id`, for the assertions only.
    fn is_pending(&self, id: u64) -> bool {
        self.pending.iter().any(is_equal_to!(&id))
    }
}

impl<C: Commit, R: Commit> Permit<C, R> {
    /// The number this permit holds.
    #[inline]
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The frontier as it stood when this permit was drawn. It may have moved
    /// since; it is only a sample taken while the lock was already held.
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

    /// What a test counter recorded being asked to do.
    type Log = Arc<Mutex<Vec<u64>>>;

    /// A counter that records what it was asked to commit and release.
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

    /// The whole point: a number that has been handed out is not readable
    /// until the write holding it is done.
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

    /// A later write finishing first must not carry the frontier past the
    /// earlier one that is still in flight.
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

    /// Retiring the oldest while a newer one is outstanding moves the frontier
    /// only as far as that newer one allows.
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

    /// A permit sees the frontier as it was when it was drawn, which is what a
    /// writer that also wants to read is asking for.
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

    /// A number that could not be persisted was never handed out, so the
    /// counter has not moved.
    #[test]
    fn a_failed_commit_draws_nothing() {
        let counter = Counter::new(7, |_| Err(crate::err!("no")), |_| Ok(()));

        assert!(counter.next().is_err());
        assert_eq!(counter.dispatched(), 7);
        assert_eq!(counter.current(), 7);
    }
}
