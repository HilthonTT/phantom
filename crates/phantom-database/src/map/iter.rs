//! The one iteration path every stream and key-stream is built from.
//!
//! All of them do the same three things and differ only in two compile-time
//! parameters — which direction the cursor runs, and whether it yields keys or
//! whole entries:
//!
//! 1. Position a cursor, on the calling thread if the block cache can answer
//!    and on [`the pool`](crate::pool) if it cannot.
//! 2. Step it, which the engine's readahead is expected to answer from memory.
//! 3. Stop where the caller's bound says to.
//!
//! The public surface in [`keys`](super::keys) and [`stream`](super::stream)
//! is these with the parameters filled in.

use std::{convert::AsRef, sync::Arc};

use futures::{FutureExt, Stream, StreamExt, TryFutureExt, TryStreamExt, future};
use phantom_core::{Result, implement};
use rocksdb::{Direction, ReadOptions};
use tokio::task;

use crate::{
    cursor::{Cursor, Fetch, State},
    keyval::{Key, KeyVal},
    pool::{self, Seek},
};

/// Streams from `from`, or from the near end of the column when there is no
/// bound.
///
/// `T` is what each step yields and `REV` which way the cursor runs; both are
/// resolved at compile time, so a step costs no branch on either.
#[implement(super::Map)]
pub(super) fn iter_from<'a, T, const REV: bool>(
    self: &'a Arc<Self>,
    from: Option<&[u8]>,
) -> impl Stream<Item = Result<T>> + Send + use<'a, T, REV>
where
    T: Fetch<'a> + Send + 'a,
{
    self.iter_bounded::<T, REV>(from, None)
}

/// Streams the entries whose keys start with `prefix`.
///
/// The prefix is applied twice over. As an upper bound on the cursor it lets
/// the engine skip whole files that cannot hold a match, and — running
/// backwards — it is what positions the cursor at the *end* of the prefix's
/// range rather than before its beginning. As a predicate on the stream it is
/// what stops the cursor once it walks out of that range at the other end.
///
/// `prefix` is taken by value so that a caller which serialized it into a
/// buffer of its own can hand that buffer over rather than keep it alive.
#[implement(super::Map)]
pub(super) fn iter_prefix<'a, T, P, const REV: bool>(
    self: &'a Arc<Self>,
    prefix: P,
) -> impl Stream<Item = Result<T>> + Send + use<'a, T, P, REV>
where
    T: Fetch<'a> + AsKey + Send + 'a,
    P: AsRef<[u8]> + Send + 'a,
{
    let upper = prefix_upper_bound(prefix.as_ref());

    // Forward, the range begins at the prefix. Backwards it ends just below
    // the upper bound, which the bound itself positions the cursor at, so
    // there is nothing to seek to.
    let from = (!REV).then(|| prefix.as_ref().to_vec());

    self.iter_bounded::<T, REV>(from.as_deref(), upper)
        .try_take_while(move |item| future::ok(item.as_key().starts_with(prefix.as_ref())))
}

/// [`Self::iter_from`] with an exclusive upper bound on the cursor.
#[implement(super::Map)]
fn iter_bounded<'a, T, const REV: bool>(
    self: &'a Arc<Self>,
    from: Option<&[u8]>,
    upper: Option<Vec<u8>>,
) -> impl Stream<Item = Result<T>> + Send + use<'a, T, REV>
where
    T: Fetch<'a> + Send + 'a,
{
    let state = State::new(self, self.iter_options(upper.clone(), false));

    if self.iter_is_cached::<REV>(from, upper) {
        let state = state.init::<REV>(from);

        // Positioned without I/O, and the steps after it will not block
        // either, so this stream will never yield to the scheduler on its
        // own. Spending the cooperative budget once up front is what stops a
        // long cached iteration from monopolising its tokio worker.
        return task::consume_budget()
            .map(move |()| Cursor::<'a, T, REV>::from(state))
            .into_stream()
            .flatten()
            .boxed();
    }

    let seek = Seek {
        map: self.clone(),
        dir: if REV {
            Direction::Reverse
        } else {
            Direction::Forward
        },
        key: from.map(Into::into),
        state: pool::send_seek(state),
        res: None,
    };

    self.db
        .pool
        .execute_iter(seek)
        .ok_into::<Cursor<'a, T, REV>>()
        .into_stream()
        .try_flatten()
        .boxed()
}

/// Whether the cursor can be positioned without going to disk.
///
/// Asks for a cache-only seek and reads the status: the engine reports an
/// incomplete read rather than failing, which is the signal to submit the seek
/// to the pool instead. The cursor built here is discarded — only its status
/// was wanted — and the one the caller goes on to use is positioned again.
#[implement(super::Map)]
#[tracing::instrument(name = "cached", level = "trace", skip_all, fields(%self))]
fn iter_is_cached<const REV: bool>(
    self: &Arc<Self>,
    from: Option<&[u8]>,
    upper: Option<Vec<u8>>,
) -> bool {
    let opts = self.iter_options(upper, true);

    !State::new(self, opts).init::<REV>(from).is_incomplete()
}

#[implement(super::Map)]
fn iter_options(&self, upper: Option<Vec<u8>>, cached: bool) -> ReadOptions {
    let mut opts = if cached {
        super::cache_iter_options_default(&self.db)
    } else {
        super::iter_options_default(&self.db)
    };

    if let Some(upper) = upper {
        opts.set_iterate_upper_bound(upper);
    }

    opts
}

/// The first key that sorts above everything starting with `prefix`.
///
/// Found by incrementing the last byte below `0xFF` and dropping whatever
/// followed it. `None` where every byte is already `0xFF`, in which case
/// nothing sorts above the prefix and there is no bound to apply.
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let len = prefix.iter().rposition(|&byte| byte < u8::MAX)?;
    let mut upper = prefix[..=len].to_vec();
    upper[len] += 1;

    Some(upper)
}

/// The key half of whatever a cursor yields, so that a prefix bound can be
/// applied without knowing which of the two item shapes it is applied to.
pub(super) trait AsKey {
    fn as_key(&self) -> Key<'_>;
}

impl AsKey for Key<'_> {
    #[inline]
    fn as_key(&self) -> Key<'_> {
        self
    }
}

impl AsKey for KeyVal<'_> {
    #[inline]
    fn as_key(&self) -> Key<'_> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What makes the bound exclusive and correct: it must sort above every
    /// key with the prefix and below every key without it.
    #[test]
    fn the_upper_bound_sits_just_past_the_prefix() {
        assert_eq!(prefix_upper_bound(b"abc").as_deref(), Some(&b"abd"[..]));
        assert_eq!(prefix_upper_bound(b"ab\xFF").as_deref(), Some(&b"ac"[..]));
        assert_eq!(
            prefix_upper_bound(b"a\xFF\xFF").as_deref(),
            Some(&b"b"[..]),
            "trailing maxima are dropped, not carried"
        );
    }

    /// A key ending in the record separator is the ordinary case here, since
    /// that is what `Interfix` leaves behind. The bound has to clear the whole
    /// separated range without swallowing the component that follows it.
    #[test]
    fn a_separator_terminated_prefix_bounds_its_own_range() {
        let prefix = b"!room\xFF";
        let upper = prefix_upper_bound(prefix).expect("bounded");

        for within in [&b"!room\xFF"[..], b"!room\xFF\xFF", b"!room\xFF\xFFevent"] {
            assert!(
                &upper[..] > within,
                "{upper:?} should sort above {within:?}, which is in the range"
            );
        }

        // A sibling component sorts *below* the range rather than above it,
        // because the separator is the highest byte there is.
        assert!(b"!rooms"[..] < prefix[..], "a sibling precedes the range");

        // And the bound is the least key above the range: nothing sorts
        // between the two.
        assert_eq!(upper, b"!roon", "one past the last byte below the maximum");
    }

    /// Nothing sorts above a prefix of all-maximum bytes, so there is no
    /// bound to give the cursor and it runs to the end of the column.
    #[test]
    fn an_unbounded_prefix_has_no_upper_bound() {
        assert_eq!(prefix_upper_bound(b"\xFF\xFF"), None);
        assert_eq!(prefix_upper_bound(b""), None);
    }
}
