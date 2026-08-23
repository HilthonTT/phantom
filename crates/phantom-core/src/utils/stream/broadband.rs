//! Broadband stream combinator extensions to [`futures::Stream`].

use std::convert::identity;

use futures::{
    Future,
    stream::{Stream, StreamExt},
};

use super::{ReadyExt, band::width};

/// Concurrency extensions to augment [`futures::StreamExt`]. `broad_`
/// combinators run up to [`automatic_width`] futures at once and yield results
/// as they finish, so the output order is not the input order. Where order
/// matters, use the `wide_` combinators in [`super::WidebandExt`] instead.
pub trait BroadbandExt<Item>
where
    Self: Stream<Item = Item> + Send + Sized,
{
    fn broadn_all<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send;

    fn broadn_any<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send;

    /// Concurrent `filter_map()`; unordered results
    fn broadn_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send;

    fn broadn_flat_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Stream<Item = U> + Send + Unpin,
        U: Send;

    fn broadn_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send;

    #[inline]
    fn broad_all<F, Fut>(self, f: F) -> impl Future<Output = bool> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send,
    {
        self.broadn_all(None, f)
    }

    #[inline]
    fn broad_any<F, Fut>(self, f: F) -> impl Future<Output = bool> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send,
    {
        self.broadn_any(None, f)
    }

    #[inline]
    fn broad_filter_map<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send,
    {
        self.broadn_filter_map(None, f)
    }

    #[inline]
    fn broad_flat_map<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Stream<Item = U> + Send + Unpin,
        U: Send,
    {
        self.broadn_flat_map(None, f)
    }

    #[inline]
    fn broad_then<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send,
    {
        self.broadn_then(None, f)
    }
}

impl<Item, S> BroadbandExt<Item> for S
where
    S: Stream<Item = Item> + Send + Sized,
{
    #[inline]
    fn broadn_all<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send,
    {
        self.map(f).buffer_unordered(width(n)).ready_all(identity)
    }

    #[inline]
    fn broadn_any<F, Fut, N>(self, n: N, f: F) -> impl Future<Output = bool> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = bool> + Send,
    {
        self.map(f).buffer_unordered(width(n)).ready_any(identity)
    }

    #[inline]
    fn broadn_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send,
    {
        self.map(f)
            .buffer_unordered(width(n))
            .ready_filter_map(identity)
    }

    #[inline]
    fn broadn_flat_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Stream<Item = U> + Send + Unpin,
        U: Send,
    {
        self.flat_map_unordered(width(n), f)
    }

    #[inline]
    fn broadn_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send,
    {
        self.map(f).buffer_unordered(width(n))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use futures::{StreamExt, future::ready};

    use super::*;
    use crate::utils::IterStream;

    #[tokio::test]
    async fn results_are_yielded_as_they_finish() {
        // Later items sleep for less, so a concurrent run reverses them and a
        // sequential one would not.
        let out: Vec<u8> = (1_u8..=4)
            .stream()
            .broad_then(|item| async move {
                tokio::time::sleep(Duration::from_millis(u64::from(5 - item) * 20)).await;
                item
            })
            .collect()
            .await;

        assert_eq!(out, [4, 3, 2, 1]);
    }

    #[tokio::test]
    async fn concurrency_is_bounded_by_the_width() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let (running, peak) = (running.clone(), peak.clone());
        let out: Vec<usize> = (0_usize..8)
            .stream()
            .broadn_then(2, |item| {
                let (running, peak) = (running.clone(), peak.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst).saturating_add(1);
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    item
                }
            })
            .collect()
            .await;

        assert_eq!(out.len(), 8);
        assert!(peak.load(Ordering::SeqCst) <= 2, "width was exceeded");
    }

    #[tokio::test]
    async fn a_zero_width_still_makes_progress() {
        // `buffer_unordered(0)` never polls anything; a caller passing a
        // computed zero should not deadlock the stream.
        let out: Vec<u8> = (1_u8..=3).stream().broadn_then(0, ready).collect().await;

        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn predicates_short_circuit() {
        assert!((1_u8..=4).stream().broad_any(|item| ready(item == 3)).await);
        assert!(!(1_u8..=4).stream().broad_all(|item| ready(item > 2)).await);
    }

    #[tokio::test]
    async fn filter_map_and_flat_map_reach_every_item() {
        let mut kept: Vec<u8> = (1_u8..=4)
            .stream()
            .broad_filter_map(|item| ready((item % 2 == 0).then_some(item)))
            .collect()
            .await;
        kept.sort_unstable();
        assert_eq!(kept, [2, 4]);

        let mut flat: Vec<u8> = (1_u8..=3)
            .stream()
            .broad_flat_map(|item| vec![item, item].stream().boxed())
            .collect()
            .await;
        flat.sort_unstable();
        assert_eq!(flat, [1, 1, 2, 2, 3, 3]);
    }
}
