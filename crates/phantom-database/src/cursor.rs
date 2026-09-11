use std::{marker::PhantomData, pin::Pin, sync::Arc};

use futures::{
    Stream,
    stream::FusedStream,
    task::{Context, Poll},
};
use phantom_core::{Result, exchange};
use rocksdb::{DBRawIteratorWithThreadMode, ReadOptions};

use crate::{
    engine::Db,
    engine::error::{is_incomplete, map_err},
    keyval::{Key, KeyVal, Slice},
    map::Map,
};

pub(crate) const FORWARD: bool = false;
pub(crate) const REVERSE: bool = true;

type Inner<'a> = DBRawIteratorWithThreadMode<'a, Db>;

pub(crate) struct State<'a> {
    inner: Inner<'a>,

    seek: bool,

    init: bool,
}

pub(crate) struct Cursor<'a, T, const REV: bool> {
    state: State<'a>,

    _item: PhantomData<fn() -> T>,
}

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
    #[inline]
    pub(crate) fn new(map: &'a Arc<Map>, opts: ReadOptions) -> Self {
        Self {
            inner: map.db().db.raw_iterator_cf_opt(&map.cf(), opts),
            init: true,
            seek: false,
        }
    }

    #[inline]
    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn init<const REV: bool>(mut self, from: Option<Key<'_>>) -> Self {
        debug_assert!(self.init, "cursor was already stepped");
        debug_assert!(!self.seek, "cursor was already positioned");

        match from {
            Some(key) if REV => self.inner.seek_for_prev(key),
            Some(key) => self.inner.seek(key),
            None if REV => self.inner.seek_to_last(),
            None => self.inner.seek_to_first(),
        }

        self.seek = true;
        self
    }

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
    #[inline]
    fn get(&self) -> Option<Result<T>> {
        T::fetch(&self.state)
            .map(Ok)
            .or_else(|| self.state.status().map(map_err).map(Err))
    }
}

impl<'a, T: Fetch<'a>, const REV: bool> Stream for Cursor<'a, T, REV> {
    type Item = Result<T>;

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

#[inline]
#[allow(unsafe_code)]
fn slice_longevity<'a, 'b: 'a>(item: &'a Slice) -> &'b Slice {
    unsafe { std::mem::transmute(item) }
}
