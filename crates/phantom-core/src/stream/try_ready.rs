//! Synchronous combinator extensions to [`futures::TryStream`].
//!
//! The fallible counterparts of [`super::ReadyExt`]: the same folding-in of
//! `ready(..)`, for combinators whose closures return a [`Result`].
#![allow(clippy::type_complexity)]

use futures::{
    future::{Ready, ready},
    stream::{AndThen, TryFilterMap, TryFold, TryForEach, TryStream, TryStreamExt, TryTakeWhile},
};

use crate::Result;

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait TryReadyExt<T, E, S>
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
    Self: TryStream + Send + Sized,
{
    fn ready_and_then<U, F>(
        self,
        f: F,
    ) -> AndThen<Self, Ready<Result<U, E>>, impl FnMut(S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(S::Ok) -> Result<U, E>;

    fn ready_try_filter_map<F, U>(
        self,
        f: F,
    ) -> TryFilterMap<
        Self,
        Ready<Result<Option<U>, E>>,
        impl FnMut(S::Ok) -> Ready<Result<Option<U>, E>>,
    >
    where
        F: Fn(S::Ok) -> Result<Option<U>, E>;

    fn ready_try_fold<U, F>(
        self,
        init: U,
        f: F,
    ) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(U, S::Ok) -> Result<U, E>;

    fn ready_try_fold_default<U, F>(
        self,
        f: F,
    ) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(U, S::Ok) -> Result<U, E>,
        U: Default;

    fn ready_try_for_each<F>(
        self,
        f: F,
    ) -> TryForEach<Self, Ready<Result<(), E>>, impl FnMut(S::Ok) -> Ready<Result<(), E>>>
    where
        F: FnMut(S::Ok) -> Result<(), E>;

    fn ready_try_take_while<F>(
        self,
        f: F,
    ) -> TryTakeWhile<Self, Ready<Result<bool, E>>, impl FnMut(&S::Ok) -> Ready<Result<bool, E>>>
    where
        F: Fn(&S::Ok) -> Result<bool, E>;
}

impl<T, E, S> TryReadyExt<T, E, S> for S
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
    Self: TryStream + Send + Sized,
{
    #[inline]
    fn ready_and_then<U, F>(
        self,
        f: F,
    ) -> AndThen<Self, Ready<Result<U, E>>, impl FnMut(S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(S::Ok) -> Result<U, E>,
    {
        self.and_then(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_try_filter_map<F, U>(
        self,
        f: F,
    ) -> TryFilterMap<
        Self,
        Ready<Result<Option<U>, E>>,
        impl FnMut(S::Ok) -> Ready<Result<Option<U>, E>>,
    >
    where
        F: Fn(S::Ok) -> Result<Option<U>, E>,
    {
        self.try_filter_map(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_try_fold<U, F>(
        self,
        init: U,
        f: F,
    ) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(U, S::Ok) -> Result<U, E>,
    {
        self.try_fold(init, move |a, t| ready(f(a, t)))
    }

    #[inline]
    fn ready_try_fold_default<U, F>(
        self,
        f: F,
    ) -> TryFold<Self, Ready<Result<U, E>>, U, impl FnMut(U, S::Ok) -> Ready<Result<U, E>>>
    where
        F: Fn(U, S::Ok) -> Result<U, E>,
        U: Default,
    {
        self.ready_try_fold(U::default(), f)
    }

    #[inline]
    fn ready_try_for_each<F>(
        self,
        mut f: F,
    ) -> TryForEach<Self, Ready<Result<(), E>>, impl FnMut(S::Ok) -> Ready<Result<(), E>>>
    where
        F: FnMut(S::Ok) -> Result<(), E>,
    {
        self.try_for_each(move |t| ready(f(t)))
    }

    #[inline]
    fn ready_try_take_while<F>(
        self,
        f: F,
    ) -> TryTakeWhile<Self, Ready<Result<bool, E>>, impl FnMut(&S::Ok) -> Ready<Result<bool, E>>>
    where
        F: Fn(&S::Ok) -> Result<bool, E>,
    {
        self.try_take_while(move |t| ready(f(t)))
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;

    use super::*;
    use crate::{Error, stream::IterStream};

    fn digits() -> impl TryStream<Ok = u8, Error = Error, Item = Result<u8>> + Send {
        (1_u8..=4).try_stream()
    }

    #[tokio::test]
    async fn and_then_maps_the_ok_side() {
        let out: Vec<u8> = digits()
            .ready_and_then(|item| Ok(item * 2))
            .try_collect()
            .await
            .expect("no errors");

        assert_eq!(out, [2, 4, 6, 8]);
    }

    #[tokio::test]
    async fn an_error_from_the_closure_ends_the_stream() {
        let out: Result<Vec<u8>> = digits()
            .ready_and_then(|item| {
                if item == 3 {
                    return Err(crate::err!("boom"));
                }

                Ok(item)
            })
            .try_collect()
            .await;

        assert_eq!(out.expect_err("failed").message(), "boom");
    }

    #[tokio::test]
    async fn filter_map_fold_and_take_while_compose() {
        let evens: Vec<u8> = digits()
            .ready_try_filter_map(|item| Ok((item % 2 == 0).then_some(item)))
            .try_collect()
            .await
            .expect("no errors");
        assert_eq!(evens, [2, 4]);

        let sum: u32 = digits()
            .ready_try_fold_default(|sum: u32, item| Ok(sum + u32::from(item)))
            .await
            .expect("no errors");
        assert_eq!(sum, 10);

        let taken: Vec<u8> = digits()
            .ready_try_take_while(|item| Ok(*item < 3))
            .try_collect()
            .await
            .expect("no errors");
        assert_eq!(taken, [1, 2]);

        let mut seen = Vec::new();
        digits()
            .ready_try_for_each(|item| {
                seen.push(item);
                Ok(())
            })
            .await
            .expect("no errors");
        assert_eq!(seen, [1, 2, 3, 4]);
    }
}
