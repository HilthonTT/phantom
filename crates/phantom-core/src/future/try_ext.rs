//! Extended external extensions to futures::TryFutureExt
#![allow(clippy::type_complexity)]
// is_ok() has to consume *self rather than borrow. This extension is for a
// caller only ever caring about result status while discarding all contents.
#![allow(clippy::wrong_self_convention)]

use std::marker::Unpin;

use futures::{
    TryFuture, TryFutureExt, future,
    future::{MapOkOrElse, TrySelect, UnwrapOrElse},
};

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait TryExt<T, E>
where
    Self: TryFuture<Ok = T, Error = E> + Send,
{
    fn is_err(
        self,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> bool, impl FnOnce(Self::Error) -> bool>
    where
        Self: Sized;

    #[allow(clippy::wrong_self_convention)]
    fn is_ok(
        self,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> bool, impl FnOnce(Self::Error) -> bool>
    where
        Self: Sized;

    fn map_ok_or<U, F>(
        self,
        default: U,
        f: F,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> U, impl FnOnce(Self::Error) -> U>
    where
        F: FnOnce(Self::Ok) -> U,
        Self: Send + Sized;

    fn ok(
        self,
    ) -> MapOkOrElse<
        Self,
        impl FnOnce(Self::Ok) -> Option<Self::Ok>,
        impl FnOnce(Self::Error) -> Option<Self::Ok>,
    >
    where
        Self: Sized;

    fn try_until<A, B, F>(self, f: F) -> TrySelect<A, B>
    where
        Self: Sized,
        F: FnOnce() -> B,
        A: TryFuture<Ok = Self::Ok> + From<Self> + Send + Unpin,
        B: TryFuture<Ok = (), Error = Self::Error> + Send + Unpin;

    fn unwrap_or(
        self,
        default: Self::Ok,
    ) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
    where
        Self: Sized;

    fn unwrap_or_default(self) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
    where
        Self: Sized,
        Self::Ok: Default;
}

impl<T, E, Fut> TryExt<T, E> for Fut
where
    Fut: TryFuture<Ok = T, Error = E> + Send,
{
    #[inline]
    fn is_err(
        self,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> bool, impl FnOnce(Self::Error) -> bool>
    where
        Self: Sized,
    {
        self.map_ok_or(true, |_| false)
    }

    #[inline]
    fn is_ok(
        self,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> bool, impl FnOnce(Self::Error) -> bool>
    where
        Self: Sized,
    {
        self.map_ok_or(false, |_| true)
    }

    #[inline]
    fn map_ok_or<U, F>(
        self,
        default: U,
        f: F,
    ) -> MapOkOrElse<Self, impl FnOnce(Self::Ok) -> U, impl FnOnce(Self::Error) -> U>
    where
        F: FnOnce(Self::Ok) -> U,
        Self: Send + Sized,
    {
        self.map_ok_or_else(|_| default, f)
    }

    #[inline]
    fn ok(
        self,
    ) -> MapOkOrElse<
        Self,
        impl FnOnce(Self::Ok) -> Option<Self::Ok>,
        impl FnOnce(Self::Error) -> Option<Self::Ok>,
    >
    where
        Self: Sized,
    {
        self.map_ok_or(None, Some)
    }

    #[inline]
    fn try_until<A, B, F>(self, f: F) -> TrySelect<A, B>
    where
        Self: Sized,
        F: FnOnce() -> B,
        A: TryFuture<Ok = Self::Ok> + From<Self> + Send + Unpin,
        B: TryFuture<Ok = (), Error = Self::Error> + Send + Unpin,
    {
        future::try_select(self.into(), f())
    }

    #[inline]
    fn unwrap_or(
        self,
        default: Self::Ok,
    ) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
    where
        Self: Sized,
    {
        self.unwrap_or_else(move |_| default)
    }

    #[inline]
    fn unwrap_or_default(self) -> UnwrapOrElse<Self, impl FnOnce(Self::Error) -> Self::Ok>
    where
        Self: Sized,
        Self::Ok: Default,
    {
        self.unwrap_or(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use futures::{executor::block_on, future::ready};

    use super::TryExt;

    fn ok() -> futures::future::Ready<Result<u32, ()>> {
        ready(Ok(7))
    }
    fn err() -> futures::future::Ready<Result<u32, ()>> {
        ready(Err(()))
    }

    #[test]
    fn status() {
        assert!(block_on(ok().is_ok()));
        assert!(!block_on(ok().is_err()));
        assert!(block_on(err().is_err()));
        assert!(!block_on(err().is_ok()));
    }

    #[test]
    fn discarding_the_error() {
        assert_eq!(block_on(ok().ok()), Some(7));
        assert_eq!(block_on(err().ok()), None);
        assert_eq!(block_on(ok().unwrap_or(1)), 7);
        assert_eq!(block_on(err().unwrap_or(1)), 1);
        assert_eq!(block_on(err().unwrap_or_default()), 0);
    }

    #[test]
    fn map_ok_or_uses_default_on_error() {
        assert_eq!(block_on(ok().map_ok_or(0, |v| v * 2)), 14);
        assert_eq!(block_on(err().map_ok_or(0, |v| v * 2)), 0);
    }
}
