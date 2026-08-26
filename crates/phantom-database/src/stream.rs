//! Turning an engine cursor into a [`futures::Stream`].
//!
//! The engine iterates through a cursor which is stepped by hand and borrows
//! into the block it is currently positioned on. A `Stream` cannot express
//! that — its `Item` has no lifetime to tie to the cursor — so the borrow is
//! extended to the cursor's own lifetime here. What that costs, and what keeps
//! it sound, is written out at [`slice_longevity`].

use std::{marker::PhantomData, pin::Pin, sync::Arc};

use futures::{
    Stream,
    stream::FusedStream,
    task::{Context, Poll},
};
use phantom_core::{Result, utils::exchange};
use rocksdb::{DBRawIteratorWithThreadMode, ReadOptions};

use crate::{
    engine::Db,
    keyval::{Key, KeyVal, Slice},
    map::Map,
    util::{is_incomplete, map_err},
};

/// The two values [`Cursor`]'s direction parameter takes, so that a callsite
/// reads as a direction rather than as a bare boolean.
pub(crate) const FORWARD: bool = false;
pub(crate) const REVERSE: bool = true;

type Inner<'a> = DBRawIteratorWithThreadMode<'a, Db>;

/// An engine cursor, and how far along it has been driven.
///
/// Split out from [`Cursor`] because the pool moves one of these across a
/// channel to be positioned on a worker thread, and the item type it will
/// eventually yield is of no interest there.
pub(crate) struct State<'a> {
    inner: Inner<'a>,

    /// Whether the cursor has been positioned yet. Until it has, stepping it
    /// would step from nowhere.
    seek: bool,

    /// Whether the next step is the first one. The first step yields the
    /// entry the cursor was positioned on rather than moving off it.
    init: bool,
}

/// A [`State`] together with which end of an entry it yields.
///
/// `T` is the item — a key, or a key and its value — and `REV` the direction.
/// Both are compile-time, so stepping the cursor costs no branch on either.
pub(crate) struct Cursor<'a, T, const REV: bool> {
    state: State<'a>,

    /// `fn() -> T` rather than `T`: the cursor produces items, it does not
    /// hold one, and marking it as holding one would make whether it can be
    /// moved or sent depend on an item type that is only ever a borrow.
    _item: PhantomData<fn() -> T>,
}

/// What a cursor yields at its current position.
///
/// Implemented for the two item shapes rather than being a method on
/// [`State`] so that [`Cursor`] can be generic over them.
pub(crate) trait Fetch<'a>: Sized {
    fn fetch(state: &State<'a>) -> Option<Self>;
}

impl<'a> Fetch<'a> for Key<'a> {
    #[inline]
    fn fetch(state: &State<'a>) -> Option<Self> {
        state.inner.key().map(slice_longevity)
    }
}

impl<'a> Fetch<'a> for KeyVal<'a> {
    #[inline]
    fn fetch(state: &State<'a>) -> Option<Self> {
        state.inner.item().map(keyval_longevity)
    }
}

impl<'a> State<'a> {
    /// A cursor over `map`, not yet positioned.
    #[inline]
    pub(crate) fn new(map: &'a Arc<Map>, opts: ReadOptions) -> Self {
        Self {
            inner: map.db().db.raw_iterator_cf_opt(&map.cf(), opts),
            init: true,
            seek: false,
        }
    }

    /// Positions the cursor at `from`, or at the end of the column the
    /// iteration starts from when there is no lower or upper bound.
    ///
    /// This is the step that may block on I/O, which is why it is separated
    /// from the stepping that follows: the pool runs this on a worker thread
    /// and sends the positioned cursor back, after which the engine's own
    /// readahead is expected to keep the rest off the disk.
    #[inline]
    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn init<const REV: bool>(mut self, from: Option<Key<'_>>) -> Self {
        debug_assert!(self.init, "cursor was already stepped");
        debug_assert!(!self.seek, "cursor was already positioned");

        match from {
            // `seek` lands on the first key at or after `from`, and
            // `seek_for_prev` on the last at or before it, so each direction
            // starts on the entry nearest the bound on the side it will move
            // away from.
            Some(key) if REV => self.inner.seek_for_prev(key),
            Some(key) => self.inner.seek(key),
            None if REV => self.inner.seek_to_last(),
            None => self.inner.seek_to_first(),
        }

        self.seek = true;
        self
    }

    /// Advances one entry, or positions the cursor if [`Self::init`] never ran.
    #[inline]
    #[cfg_attr(unabridged, tracing::instrument(level = "trace", skip_all))]
    fn step<const REV: bool>(&mut self) {
        if !exchange(&mut self.init, false) {
            if REV {
                self.inner.prev()
            } else {
                self.inner.next()
            }
        } else if !self.seek {
            if REV {
                self.inner.seek_to_last();
            } else {
                self.inner.seek_to_first();
            }
        }
    }

    /// Whether the engine declined to answer rather than running out of
    /// entries, which is how a cache-only cursor reports a miss.
    #[inline]
    pub(crate) fn is_incomplete(&self) -> bool {
        matches!(self.status(), Some(ref e) if is_incomplete(e))
    }

    #[inline]
    fn status(&self) -> Option<rocksdb::Error> {
        self.inner.status().err()
    }

    #[inline]
    fn valid(&self) -> bool {
        self.inner.valid()
    }
}

impl<'a, T, const REV: bool> From<State<'a>> for Cursor<'a, T, REV> {
    #[inline]
    fn from(state: State<'a>) -> Self {
        Self {
            state,
            _item: PhantomData,
        }
    }
}

impl<'a, T: Fetch<'a>, const REV: bool> Cursor<'a, T, REV> {
    /// The entry at the current position, or the failure that ended the
    /// iteration. Running out of entries is neither, and ends the stream.
    #[inline]
    fn get(&self) -> Option<Result<T>> {
        T::fetch(&self.state)
            .map(Ok)
            .or_else(|| self.state.status().map(map_err).map(Err))
    }
}

impl<'a, T: Fetch<'a>, const REV: bool> Stream for Cursor<'a, T, REV> {
    type Item = Result<T>;

    /// Never pends: the cursor was positioned before the stream was built, and
    /// the engine's readahead is what keeps the steps after that off the disk.
    fn poll_next(mut self: Pin<&mut Self>, _ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.state.step::<REV>();

        Poll::Ready(self.get())
    }
}

impl<'a, T: Fetch<'a>, const REV: bool> FusedStream for Cursor<'a, T, REV> {
    #[inline]
    fn is_terminated(&self) -> bool {
        !self.state.init && !self.state.valid()
    }
}

#[inline]
fn keyval_longevity<'a, 'b: 'a>(item: KeyVal<'a>) -> KeyVal<'b> {
    (slice_longevity(item.0), slice_longevity(item.1))
}

/// Extends a borrow into the cursor's current position to the lifetime of the
/// cursor itself.
///
/// # Safety
///
/// The engine only guarantees the slice until the cursor next moves, and this
/// claims it for as long as the cursor lives. Nothing in the type system
/// enforces the difference, so the rule is a convention: **an item yielded by
/// one of these streams must not outlive the step that produced it.**
///
/// In practice the combinators uphold it — a closure passed to `map` or
/// `for_each` cannot let the borrow escape, because the compiler still checks
/// that against the closure's own signature. What it will not catch is
/// collecting the stream: `.collect::<Vec<_>>()` on a stream of borrows
/// compiles and yields a vector of dangling slices. Call
/// `.map(ToOwned::to_owned)` first, or deserialize into an owned type.
///
/// The alternative is a `Stream` whose `Item` may borrow from the stream,
/// which the trait cannot express; this goes away if that changes.
#[inline]
#[allow(unsafe_code)]
fn slice_longevity<'a, 'b: 'a>(item: &'a Slice) -> &'b Slice {
    // SAFETY: see the contract above; upheld by the callers of the map layer's
    // iteration methods, not by this function.
    unsafe { std::mem::transmute(item) }
}
