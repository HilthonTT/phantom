//! Intersecting sorted sequences.
//!
//! The inputs are already in order — they come off the database in key order —
//! so the intersection is a merge rather than a hash join, and streams out
//! without ever holding both sides in memory.

use std::{
    cmp::{Eq, Ord},
    pin::Pin,
    sync::Arc,
};

use futures::{Stream, StreamExt};

use crate::{is_equal_to, is_less_than};

/// Intersection of sets
///
/// Outputs the set of elements common to all input sets. Inputs do not have to
/// be sorted. If inputs are sorted a more optimized function is available in
/// this suite and should be used.
pub fn intersection<Item, Iter, Iters>(mut input: Iters) -> impl Iterator<Item = Item> + Send
where
    Iters: Iterator<Item = Iter> + Clone + Send,
    Iter: Iterator<Item = Item> + Send,
    Item: Eq,
{
    input.next().into_iter().flat_map(move |first| {
        let input = input.clone();
        first.filter(move |targ| {
            input
                .clone()
                .all(|mut other| other.any(is_equal_to!(*targ)))
        })
    })
}

/// Intersection of sets
///
/// Outputs the set of elements common to all input sets. Inputs must be sorted.
pub fn intersection_sorted<Item, Iter, Iters>(mut input: Iters) -> impl Iterator<Item = Item> + Send
where
    Iters: Iterator<Item = Iter> + Clone + Send,
    Iter: Iterator<Item = Item> + Send,
    // `Send` because the peeked element is buffered in the returned iterator.
    Item: Eq + Ord + Send,
{
    input.next().into_iter().flat_map(move |first| {
        // The peekables have to outlive the closure: advancing one past the
        // elements below `targ` is only sound because whatever it stops on is
        // still there for the next, larger, `targ`.
        let mut input = input.clone().map(Iterator::peekable).collect::<Vec<_>>();
        first.filter(move |targ| {
            input.iter_mut().all(|it| {
                while it.peek().is_some_and(is_less_than!(targ)) {
                    it.next();
                }

                it.peek().is_some_and(is_equal_to!(targ))
            })
        })
    })
}

/// Intersection of sets
///
/// Outputs the set of elements common to both streams. Streams must be sorted.
pub fn intersection_sorted_stream2<Item, S>(a: S, b: S) -> impl Stream<Item = Item> + Send
where
    S: Stream<Item = Item> + Send + Unpin,
    Item: Eq + PartialOrd + Send + Sync,
{
    use tokio::sync::Mutex;

    let b = Arc::new(Mutex::new(b.peekable()));
    a.map(move |ai| (ai, b.clone()))
        .filter_map(|(ai, b)| async move {
            let mut lock = b.lock().await;
            while let Some(bi) = Pin::new(&mut *lock).next_if(|bi| *bi <= ai).await.as_ref() {
                if ai == *bi {
                    return Some(ai);
                }
            }

            None
        })
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, executor::block_on};

    use super::{intersection, intersection_sorted, intersection_sorted_stream2};

    #[test]
    fn intersection_ignores_input_order() {
        let a = [3_u32, 1, 2, 5];
        let b = [2_u32, 3];
        let got: Vec<u32> =
            intersection([a.iter().copied(), b.iter().copied()].into_iter()).collect();

        assert_eq!(got, vec![3, 2]);
    }

    #[test]
    fn intersection_of_nothing_is_empty() {
        let empty: [std::iter::Empty<u32>; 0] = [];
        assert_eq!(intersection(empty.into_iter()).count(), 0);
    }

    /// A candidate that a later set only matches after that set has been
    /// advanced must still be reported — the advanced-past element has to
    /// survive between candidates.
    #[test]
    fn intersection_sorted_keeps_matches_across_advances() {
        let a = [1_u32, 2, 3, 4];
        let b = [2_u32, 4, 6];
        let c = [2_u32, 4];
        let got: Vec<u32> = intersection_sorted(
            [a.iter().copied(), b.iter().copied(), c.iter().copied()].into_iter(),
        )
        .collect();

        assert_eq!(got, vec![2, 4]);
    }

    #[test]
    fn intersection_sorted_disjoint_is_empty() {
        let a = [1_u32, 3, 5];
        let b = [2_u32, 4, 6];
        let got: Vec<u32> =
            intersection_sorted([a.iter().copied(), b.iter().copied()].into_iter()).collect();

        assert!(got.is_empty());
    }

    #[test]
    fn intersection_sorted_stream2_matches_the_iterator_version() {
        let a = futures::stream::iter(vec![1_u32, 2, 3, 4, 7]);
        let b = futures::stream::iter(vec![2_u32, 4, 6, 7]);
        let got: Vec<u32> = block_on(intersection_sorted_stream2(a, b).collect::<Vec<_>>());

        assert_eq!(got, vec![2, 4, 7]);
    }
}
