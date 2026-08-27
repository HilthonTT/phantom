//! Synchronous combinator extensions to [`futures::Stream`].
//!
//! Most `Stream` combinators take an asynchronous predicate, but steering a
//! stream like an iterator only needs a synchronous one. Each method here is
//! its `StreamExt` counterpart with the `ready(..)` wrapping folded in, so
//! callsites are not littered with closures that immediately return.
#![allow(clippy::type_complexity)]

use futures::{
    future::{Ready, ready},
    stream::{
        All, Any, Filter, FilterMap, Fold, ForEach, Scan, SkipWhile, Stream, StreamExt, TakeWhile,
    },
};

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait ReadyExt<Item>
where
    Self: Stream<Item = Item> + Send + Sized,
{
    fn ready_all<F>(self, f: F) -> All<Self, Ready<bool>, impl FnMut(Item) -> Ready<bool>>
    where
        F: Fn(Item) -> bool;

    fn ready_any<F>(self, f: F) -> Any<Self, Ready<bool>, impl FnMut(Item) -> Ready<bool>>
    where
        F: Fn(Item) -> bool;

    fn ready_filter<'a, F>(
        self,
        f: F,
    ) -> Filter<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a;

    fn ready_filter_map<F, U>(
        self,
        f: F,
    ) -> FilterMap<Self, Ready<Option<U>>, impl FnMut(Item) -> Ready<Option<U>>>
    where
        F: Fn(Item) -> Option<U>;

    fn ready_fold<T, F>(
        self,
        init: T,
        f: F,
    ) -> Fold<Self, Ready<T>, T, impl FnMut(T, Item) -> Ready<T>>
    where
        F: Fn(T, Item) -> T;

    fn ready_fold_default<T, F>(
        self,
        f: F,
    ) -> Fold<Self, Ready<T>, T, impl FnMut(T, Item) -> Ready<T>>
    where
        F: Fn(T, Item) -> T,
        T: Default;

    fn ready_for_each<F>(self, f: F) -> ForEach<Self, Ready<()>, impl FnMut(Item) -> Ready<()>>
    where
        F: FnMut(Item);

    fn ready_take_while<'a, F>(
        self,
        f: F,
    ) -> TakeWhile<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a;

    fn ready_scan<B, T, F>(
        self,
        init: T,
        f: F,
    ) -> Scan<Self, T, Ready<Option<B>>, impl FnMut(&mut T, Item) -> Ready<Option<B>>>
    where
        F: Fn(&mut T, Item) -> Option<B>;

    fn ready_scan_each<T, F>(
        self,
        init: T,
        f: F,
    ) -> Scan<Self, T, Ready<Option<Item>>, impl FnMut(&mut T, Item) -> Ready<Option<Item>>>
    where
        F: Fn(&mut T, &Item);

    fn ready_skip_while<'a, F>(
        self,
        f: F,
    ) -> SkipWhile<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a;
}

impl<Item, S> ReadyExt<Item> for S
where
    S: Stream<Item = Item> + Send + Sized,
{
    #[inline]
    fn ready_all<F>(self, f: F) -> All<Self, Ready<bool>, impl FnMut(Item) -> Ready<bool>>
    where
        F: Fn(Item) -> bool,
    {
        self.all(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_any<F>(self, f: F) -> Any<Self, Ready<bool>, impl FnMut(Item) -> Ready<bool>>
    where
        F: Fn(Item) -> bool,
    {
        self.any(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_filter<'a, F>(
        self,
        f: F,
    ) -> Filter<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a,
    {
        self.filter(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_filter_map<F, U>(
        self,
        f: F,
    ) -> FilterMap<Self, Ready<Option<U>>, impl FnMut(Item) -> Ready<Option<U>>>
    where
        F: Fn(Item) -> Option<U>,
    {
        self.filter_map(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_fold<T, F>(
        self,
        init: T,
        f: F,
    ) -> Fold<Self, Ready<T>, T, impl FnMut(T, Item) -> Ready<T>>
    where
        F: Fn(T, Item) -> T,
    {
        self.fold(init, move |a, t| ready(f(a, t)))
    }

    #[inline]
    fn ready_fold_default<T, F>(
        self,
        f: F,
    ) -> Fold<Self, Ready<T>, T, impl FnMut(T, Item) -> Ready<T>>
    where
        F: Fn(T, Item) -> T,
        T: Default,
    {
        self.ready_fold(T::default(), f)
    }

    #[inline]
    fn ready_for_each<F>(self, mut f: F) -> ForEach<Self, Ready<()>, impl FnMut(Item) -> Ready<()>>
    where
        F: FnMut(Item),
    {
        self.for_each(move |t| {
            f(t);
            ready(())
        })
    }

    #[inline]
    fn ready_take_while<'a, F>(
        self,
        f: F,
    ) -> TakeWhile<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a,
    {
        self.take_while(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_scan<B, T, F>(
        self,
        init: T,
        f: F,
    ) -> Scan<Self, T, Ready<Option<B>>, impl FnMut(&mut T, Item) -> Ready<Option<B>>>
    where
        F: Fn(&mut T, Item) -> Option<B>,
    {
        self.scan(init, move |s, t| ready(f(s, t)))
    }

    #[inline]
    fn ready_scan_each<T, F>(
        self,
        init: T,
        f: F,
    ) -> Scan<Self, T, Ready<Option<Item>>, impl FnMut(&mut T, Item) -> Ready<Option<Item>>>
    where
        F: Fn(&mut T, &Item),
    {
        self.ready_scan(init, move |s, t| {
            f(s, &t);
            Some(t)
        })
    }

    #[inline]
    fn ready_skip_while<'a, F>(
        self,
        f: F,
    ) -> SkipWhile<Self, Ready<bool>, impl FnMut(&Item) -> Ready<bool> + 'a>
    where
        F: Fn(&Item) -> bool + 'a,
    {
        self.skip_while(move |t| ready(f(t)))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;
    use crate::stream::IterStream;

    fn digits() -> impl Stream<Item = u8> + Send {
        (1_u8..=5).stream()
    }

    #[tokio::test]
    async fn predicates_match_their_async_counterparts() {
        assert!(digits().ready_all(|item| item > 0).await);
        assert!(!digits().ready_all(|item| item > 3).await);
        assert!(digits().ready_any(|item| item == 3).await);
        assert!(!digits().ready_any(|item| item == 9).await);
    }

    #[tokio::test]
    async fn selection_keeps_stream_order() {
        let even: Vec<u8> = digits().ready_filter(|item| item % 2 == 0).collect().await;
        assert_eq!(even, [2, 4]);

        let doubled: Vec<u8> = digits()
            .ready_filter_map(|item| (item < 3).then_some(item * 2))
            .collect()
            .await;
        assert_eq!(doubled, [2, 4]);

        let taken: Vec<u8> = digits().ready_take_while(|item| *item < 3).collect().await;
        assert_eq!(taken, [1, 2]);

        let skipped: Vec<u8> = digits().ready_skip_while(|item| *item < 3).collect().await;
        assert_eq!(skipped, [3, 4, 5]);
    }

    #[tokio::test]
    async fn folds_accumulate_in_order() {
        assert_eq!(
            digits()
                .ready_fold(0_u32, |sum, item| sum + u32::from(item))
                .await,
            15
        );
        assert_eq!(
            digits()
                .ready_fold_default(|sum: u32, item| sum + u32::from(item))
                .await,
            15,
            "the default is the same starting point"
        );

        let mut seen = Vec::new();
        digits().ready_for_each(|item| seen.push(item)).await;
        assert_eq!(seen, [1, 2, 3, 4, 5]);
    }

    #[tokio::test]
    async fn scans_carry_state_between_items() {
        let running: Vec<u8> = digits()
            .ready_scan(0_u8, |sum, item| {
                *sum += item;
                Some(*sum)
            })
            .collect()
            .await;
        assert_eq!(running, [1, 3, 6, 10, 15]);

        let mut counted = 0_usize;
        let passed: Vec<u8> = digits()
            .ready_scan_each(&mut counted, |count, _| **count += 1)
            .collect()
            .await;
        assert_eq!(passed, [1, 2, 3, 4, 5], "items pass through unchanged");
        assert_eq!(counted, 5);
    }
}
