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

    let from = (!REV).then(|| prefix.as_ref().to_vec());

    self.iter_bounded::<T, REV>(from.as_deref(), upper)
        .try_take_while(move |item| future::ok(item.as_key().starts_with(prefix.as_ref())))
}

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

fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let len = prefix.iter().rposition(|&byte| byte < u8::MAX)?;
    let mut upper = prefix[..=len].to_vec();
    upper[len] += 1;

    Some(upper)
}

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

        assert!(b"!rooms"[..] < prefix[..], "a sibling precedes the range");

        assert_eq!(upper, b"!roon", "one past the last byte below the maximum");
    }

    #[test]
    fn an_unbounded_prefix_has_no_upper_bound() {
        assert_eq!(prefix_upper_bound(b"\xFF\xFF"), None);
        assert_eq!(prefix_upper_bound(b""), None);
    }
}
