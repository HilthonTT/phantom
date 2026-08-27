//! Wideband stream combinator extensions to [`futures::Stream`].

use std::convert::identity;

use futures::{
    Future,
    stream::{Stream, StreamExt},
};

use super::{ReadyExt, band::width};

/// Concurrency extensions to augment [`futures::StreamExt`]. `wide_`
/// combinators run several futures at once like the `broad_` ones in
/// [`super::BroadbandExt`], but yield their results in the input's order, at
/// the cost of holding a finished result until the ones before it are done.
pub trait WidebandExt<Item>
where
    Self: Stream<Item = Item> + Send + Sized,
{
    /// Concurrent `filter_map()`; ordered results
    fn widen_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send;

    fn widen_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send;

    #[inline]
    fn wide_filter_map<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send,
    {
        self.widen_filter_map(None, f)
    }

    #[inline]
    fn wide_then<F, Fut, U>(self, f: F) -> impl Stream<Item = U> + Send
    where
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send,
    {
        self.widen_then(None, f)
    }
}

impl<Item, S> WidebandExt<Item> for S
where
    S: Stream<Item = Item> + Send + Sized,
{
    #[inline]
    fn widen_filter_map<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = Option<U>> + Send,
        U: Send,
    {
        self.map(f).buffered(width(n)).ready_filter_map(identity)
    }

    #[inline]
    fn widen_then<F, Fut, U, N>(self, n: N, f: F) -> impl Stream<Item = U> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Item) -> Fut + Send,
        Fut: Future<Output = U> + Send,
        U: Send,
    {
        self.map(f).buffered(width(n))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{StreamExt, future::ready};

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn results_keep_the_input_order() {
        // The same workload the broadband test uses, where the futures finish
        // in reverse; here that must not show up in the output.
        let out: Vec<u8> = (1_u8..=4)
            .stream()
            .wide_then(|item| async move {
                tokio::time::sleep(Duration::from_millis(u64::from(5 - item) * 20)).await;
                item
            })
            .collect()
            .await;

        assert_eq!(out, [1, 2, 3, 4]);
    }

    #[tokio::test]
    async fn filter_map_drops_items_without_disturbing_the_order() {
        let out: Vec<u8> = (1_u8..=6)
            .stream()
            .widen_filter_map(2, |item| ready((item % 2 == 0).then_some(item)))
            .collect()
            .await;

        assert_eq!(out, [2, 4, 6]);
    }
}
