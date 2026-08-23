//! Broadband combinator extensions to [`futures::TryStream`].

use futures::{TryFuture, TryStream, TryStreamExt};

use super::band::width;
use crate::Result;

/// Concurrency extensions to augment [`futures::TryStreamExt`]. `broad_`
/// combinators produce out-of-order results; the first error ends the stream.
pub trait TryBroadbandExt<T, E>
where
    Self: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
{
    fn broadn_and_then<U, F, Fut, N>(
        self,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send;

    fn broad_and_then<U, F, Fut>(
        self,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
    {
        self.broadn_and_then(None, f)
    }
}

impl<T, E, S> TryBroadbandExt<T, E> for S
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
{
    fn broadn_and_then<U, F, Fut, N>(
        self,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        F: Fn(Self::Ok) -> Fut + Send,
        Fut: TryFuture<Ok = U, Error = E, Output = Result<U, E>> + Send,
    {
        self.map_ok(f).try_buffer_unordered(width(n))
    }
}

#[cfg(test)]
mod tests {
    use futures::{TryStreamExt, future::ok};

    use super::*;
    use crate::{Error, utils::IterStream};

    #[tokio::test]
    async fn every_item_is_mapped() {
        let mut out: Vec<u8> = (1_u8..=4)
            .try_stream()
            .broad_and_then(|item| ok(item * 2))
            .try_collect()
            .await
            .expect("no errors");
        out.sort_unstable();

        assert_eq!(out, [2, 4, 6, 8]);
    }

    #[tokio::test]
    async fn an_error_is_reported() {
        let out: Result<Vec<u8>> = (1_u8..=4)
            .try_stream()
            .broadn_and_then(2, |item| async move {
                if item == 3 {
                    return Err::<u8, Error>(crate::err!("boom"));
                }

                Ok(item)
            })
            .try_collect()
            .await;

        assert_eq!(out.expect_err("failed").message(), "boom");
    }
}
