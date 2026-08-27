//! Aggregations over a [`futures::TryStream`].
#![allow(clippy::type_complexity)]

use futures::{TryStream, TryStreamExt, future, future::Ready, stream::TryTakeWhile};

use crate::Result;

/// This interface is not necessarily complete; feel free to add as-needed.
pub trait TryAggregate<T, E, S>
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
    Self: TryStream + Send + Sized,
{
    /// [`futures::StreamExt::take`] for a fallible stream, which stops at `n`
    /// successful items and passes an error through.
    fn try_take(
        self,
        n: usize,
    ) -> TryTakeWhile<
        Self,
        Ready<Result<bool, S::Error>>,
        impl FnMut(&S::Ok) -> Ready<Result<bool, S::Error>>,
    >;
}

impl<T, E, S> TryAggregate<T, E, S> for S
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + ?Sized,
    Self: TryStream + Send + Sized,
{
    #[inline]
    fn try_take(
        self,
        mut n: usize,
    ) -> TryTakeWhile<
        Self,
        Ready<Result<bool, S::Error>>,
        impl FnMut(&S::Ok) -> Ready<Result<bool, S::Error>>,
    > {
        self.try_take_while(move |_| {
            let remaining = future::ok(n > 0);
            n = n.saturating_sub(1);
            remaining
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn takes_at_most_n_items() {
        let out: Vec<u8> = (1_u8..=5)
            .try_stream()
            .try_take(2)
            .try_collect()
            .await
            .expect("no errors");

        assert_eq!(out, [1, 2]);
    }

    #[tokio::test]
    async fn taking_none_or_more_than_there_are() {
        let none: Vec<u8> = (1_u8..=5)
            .try_stream()
            .try_take(0)
            .try_collect()
            .await
            .expect("no errors");
        assert!(none.is_empty());

        let all: Vec<u8> = (1_u8..=3)
            .try_stream()
            .try_take(10)
            .try_collect()
            .await
            .expect("no errors");
        assert_eq!(all, [1, 2, 3]);
    }
}
