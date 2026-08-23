//! Wideband combinator extensions to [`futures::TryStream`].

use futures::{TryFuture, TryStream, TryStreamExt};

use super::band::width;
use crate::Result;

/// Concurrency extensions to augment [`futures::TryStreamExt`]. `wide_`
/// combinators produce in-order results; the first error ends the stream.
pub trait TryWidebandExt<T, E>
where
    Self: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
{
    fn widen_and_then<U, F, Fut, N>(
        self,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
        U: Send;

    fn wide_and_then<U, F, Fut>(
        self,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
        U: Send,
    {
        self.widen_and_then(None, f)
    }
}

impl<T, E, S> TryWidebandExt<T, E> for S
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
    E: Send,
{
    fn widen_and_then<U, F, Fut, N>(
        self,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
        U: Send,
    {
        self.map_ok(f).try_buffered(width(n))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::TryStreamExt;

    use super::*;
    use crate::utils::IterStream;

    #[tokio::test]
    async fn results_keep_the_input_order() {
        let out: Vec<u8> = (1_u8..=4)
            .try_stream()
            .wide_and_then(|item| async move {
                tokio::time::sleep(Duration::from_millis(u64::from(5 - item) * 20)).await;
                Ok(item)
            })
            .try_collect()
            .await
            .expect("no errors");

        assert_eq!(out, [1, 2, 3, 4]);
    }
}
