//! Aggregations over a [`futures::Stream`].

use std::{collections::HashMap, hash::Hash};

use futures::{Future, Stream, StreamExt};

use super::ReadyExt;

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait Aggregate<Item>
where
    Self: Stream<Item = Item> + Send + Sized,
    <Self as Stream>::Item: Send,
{
    /// Counts how many times each item occurs.
    fn counts(self) -> impl Future<Output = HashMap<Item, usize>> + Send
    where
        <Self as Stream>::Item: Eq + Hash;

    /// Counts how many items each key drawn from them occurs for.
    fn counts_by<K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
    where
        F: Fn(Item) -> K + Send,
        K: Eq + Hash + Send;

    /// [`Self::counts_by`], reserving room for `CAP` keys up front.
    fn counts_by_with_cap<const CAP: usize, K, F>(
        self,
        f: F,
    ) -> impl Future<Output = HashMap<K, usize>> + Send
    where
        F: Fn(Item) -> K + Send,
        K: Eq + Hash + Send;

    /// [`Self::counts`], reserving room for `CAP` keys up front.
    fn counts_with_cap<const CAP: usize>(self) -> impl Future<Output = HashMap<Item, usize>> + Send
    where
        <Self as Stream>::Item: Eq + Hash;

    /// [`StreamExt::fold`] starting from [`Default::default`].
    fn fold_default<T, F, Fut>(self, f: F) -> impl Future<Output = T> + Send
    where
        F: Fn(T, Item) -> Fut + Send,
        Fut: Future<Output = T> + Send,
        T: Default + Send;
}

impl<Item, S> Aggregate<Item> for S
where
    S: Stream<Item = Item> + Send + Sized,
    <Self as Stream>::Item: Send,
{
    #[inline]
    fn counts(self) -> impl Future<Output = HashMap<Item, usize>> + Send
    where
        <Self as Stream>::Item: Eq + Hash,
    {
        self.counts_with_cap::<0>()
    }

    #[inline]
    fn counts_by<K, F>(self, f: F) -> impl Future<Output = HashMap<K, usize>> + Send
    where
        F: Fn(Item) -> K + Send,
        K: Eq + Hash + Send,
    {
        self.counts_by_with_cap::<0, K, F>(f)
    }

    #[inline]
    fn counts_by_with_cap<const CAP: usize, K, F>(
        self,
        f: F,
    ) -> impl Future<Output = HashMap<K, usize>> + Send
    where
        F: Fn(Item) -> K + Send,
        K: Eq + Hash + Send,
    {
        self.map(f).counts_with_cap::<CAP>()
    }

    #[inline]
    fn counts_with_cap<const CAP: usize>(self) -> impl Future<Output = HashMap<Item, usize>> + Send
    where
        <Self as Stream>::Item: Eq + Hash,
    {
        self.ready_fold(HashMap::with_capacity(CAP), |mut counts, item| {
            // A count that reached `usize::MAX` has bigger problems than being
            // one short; saturating here keeps an unbounded stream from
            // panicking a release build into an aborted request.
            counts
                .entry(item)
                .and_modify(|count: &mut usize| *count = count.saturating_add(1))
                .or_insert(1);

            counts
        })
    }

    #[inline]
    fn fold_default<T, F, Fut>(self, f: F) -> impl Future<Output = T> + Send
    where
        F: Fn(T, Item) -> Fut + Send,
        Fut: Future<Output = T> + Send,
        T: Default + Send,
    {
        self.fold(T::default(), f)
    }
}

#[cfg(test)]
mod tests {
    use futures::future::ready;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn counts_occurrences() {
        let counts = ["a", "b", "a"].into_iter().stream().counts().await;

        assert_eq!(counts.get("a"), Some(&2));
        assert_eq!(counts.get("b"), Some(&1));
        assert_eq!(counts.len(), 2);
    }

    #[tokio::test]
    async fn counts_by_a_derived_key() {
        let counts = (1_u8..=5)
            .stream()
            .counts_by_with_cap::<2, _, _>(|item| item % 2 == 0)
            .await;

        assert_eq!(counts.get(&true), Some(&2));
        assert_eq!(counts.get(&false), Some(&3));
    }

    #[tokio::test]
    async fn fold_default_starts_from_the_default() {
        let sum: u32 = (1_u8..=4)
            .stream()
            .fold_default(|sum: u32, item| ready(sum + u32::from(item)))
            .await;

        assert_eq!(sum, 10);
    }
}
