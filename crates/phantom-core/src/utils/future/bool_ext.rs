//! Extended external extensions to futures::FutureExt

use std::marker::Unpin;

use futures::{
    Future, FutureExt, StreamExt,
    future::{ready, select_ok, try_join, try_join_all},
    stream::FuturesUnordered,
};

pub trait BoolExt
where
    Self: Future<Output = bool> + Send,
{
    fn and<B>(self, b: B) -> impl Future<Output = bool> + Send
    where
        B: Future<Output = bool> + Send,
        Self: Sized;

    fn or<B>(self, b: B) -> impl Future<Output = bool> + Send
    where
        B: Future<Output = bool> + Send + Unpin,
        Self: Sized + Unpin;
}

pub fn and<I, F>(args: I) -> impl Future<Output = bool> + Send
where
    I: Iterator<Item = F> + Send,
    F: Future<Output = bool> + Send,
{
    type Result = crate::Result<(), ()>;

    let args = args.map(|a| a.map(|a| a.then_some(()).ok_or(Result::Err(()))));

    try_join_all(args).map(|result| result.is_ok())
}

pub fn or<I, F>(args: I) -> impl Future<Output = bool> + Send
where
    I: Iterator<Item = F> + Send,
    F: Future<Output = bool> + Send,
{
    // `select_ok` panics on an empty iterator, while `FuturesUnordered` yields
    // `false` for it, matching `Iterator::any`. It also polls concurrently and
    // short-circuits on the first `true`, so it costs nothing over `select_ok`
    // and drops the `Unpin` bound the latter requires.
    args.collect::<FuturesUnordered<_>>().any(ready)
}

impl<Fut> BoolExt for Fut
where
    Fut: Future<Output = bool> + Send,
{
    #[inline]
    fn and<B>(self, b: B) -> impl Future<Output = bool> + Send
    where
        B: Future<Output = bool> + Send,
        Self: Sized,
    {
        type Result = crate::Result<(), ()>;

        let a = self.map(|a| a.then_some(()).ok_or(Result::Err(())));

        let b = b.map(|b| b.then_some(()).ok_or(Result::Err(())));

        try_join(a, b).map(|result| result.is_ok())
    }

    #[inline]
    fn or<B>(self, b: B) -> impl Future<Output = bool> + Send
    where
        B: Future<Output = bool> + Send + Unpin,
        Self: Sized + Unpin,
    {
        type Result = crate::Result<(), ()>;

        let a = self
            .map(|a| a.then_some(()).ok_or(Result::Err(())))
            .left_future();

        let b = b
            .map(|b| b.then_some(()).ok_or(Result::Err(())))
            .right_future();

        select_ok([a, b]).map(|result| result.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use futures::{
        executor::block_on,
        future::{Ready, ready},
    };

    use super::{BoolExt, and, or};

    #[test]
    fn ext_and() {
        assert!(block_on(ready(true).and(ready(true))));
        assert!(!block_on(ready(true).and(ready(false))));
        assert!(!block_on(ready(false).and(ready(true))));
        assert!(!block_on(ready(false).and(ready(false))));
    }

    #[test]
    fn ext_or() {
        assert!(block_on(ready(true).or(ready(true))));
        assert!(block_on(ready(true).or(ready(false))));
        assert!(block_on(ready(false).or(ready(true))));
        assert!(!block_on(ready(false).or(ready(false))));
    }

    #[test]
    fn free_and() {
        assert!(block_on(and([ready(true), ready(true)].into_iter())));
        assert!(!block_on(and([ready(true), ready(false)].into_iter())));
        // An empty iterator is vacuously true, matching `Iterator::all`.
        let empty = std::iter::empty::<Ready<bool>>();
        assert!(block_on(and(empty)));
    }

    #[test]
    fn free_or() {
        assert!(block_on(or([ready(false), ready(true)].into_iter())));
        assert!(!block_on(or([ready(false), ready(false)].into_iter())));
        // An empty iterator is vacuously false, matching `Iterator::any`.
        let empty = std::iter::empty::<Ready<bool>>();
        assert!(!block_on(or(empty)));
    }
}
