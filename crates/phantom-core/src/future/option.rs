#![allow(clippy::wrong_self_convention)]

use futures::{Future, FutureExt, future::OptionFuture};

pub trait OptionExt<T> {
    fn is_none_or(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send;

    fn is_some_and(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send;
}

impl<T, Fut> OptionExt<T> for OptionFuture<Fut>
where
    Fut: Future<Output = T> + Send,
    T: Send,
{
    #[inline]
    fn is_none_or(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send {
        self.map(|o| o.as_ref().is_none_or(f))
    }

    #[inline]
    fn is_some_and(self, f: impl FnOnce(&T) -> bool + Send) -> impl Future<Output = bool> + Send {
        self.map(|o| o.as_ref().is_some_and(f))
    }
}

#[cfg(test)]
mod tests {
    use futures::{executor::block_on, future::OptionFuture, future::ready};

    use super::OptionExt;

    #[test]
    fn some() {
        let some = || OptionFuture::from(Some(ready(7)));
        assert!(block_on(some().is_some_and(|v| *v == 7)));
        assert!(!block_on(some().is_none_or(|v| *v != 7)));
    }

    #[test]
    fn none() {
        let none = || OptionFuture::<futures::future::Ready<u32>>::from(None);
        assert!(!block_on(none().is_some_and(|_| true)));
        assert!(block_on(none().is_none_or(|_| false)));
    }
}
