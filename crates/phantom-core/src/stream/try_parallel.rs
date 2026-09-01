//! Parallelism combinator extensions to [`futures::TryStream`].

use futures::{TryFutureExt, stream::TryStream};
use tokio::{runtime, task::JoinError};

use super::TryBroadbandExt;
use crate::{Error, Result, sys::compute::available_parallelism};

/// Parallelism extensions to augment [`futures::TryStreamExt`].
///
/// These are for computation-bound work, where the `-band` combinators are for
/// I/O-bound work: each item is handed to a blocking thread rather than polled
/// on the runtime, and the default concurrency is the machine's compute
/// parallelism rather than the stream width. Threads come from the tokio
/// blocking pool. Results are unordered.
pub trait TryParallelExt<T, E>
where
    Self: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
    E: From<JoinError> + From<Error> + Send + 'static,
    T: Send + 'static,
{
    fn paralleln_and_then<U, F, N, H>(
        self,
        h: H,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        H: Into<Option<runtime::Handle>>,
        F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
        U: Send + 'static;

    fn parallel_and_then<U, F, H>(
        self,
        h: H,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        H: Into<Option<runtime::Handle>>,
        F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
        U: Send + 'static,
    {
        self.paralleln_and_then(h, None, f)
    }
}

impl<T, E, S> TryParallelExt<T, E> for S
where
    S: TryStream<Ok = T, Error = E, Item = Result<T, E>> + Send + Sized,
    E: From<JoinError> + From<Error> + Send + 'static,
    T: Send + 'static,
{
    fn paralleln_and_then<U, F, N, H>(
        self,
        h: H,
        n: N,
        f: F,
    ) -> impl TryStream<Ok = U, Error = E, Item = Result<U, E>> + Send
    where
        N: Into<Option<usize>>,
        H: Into<Option<runtime::Handle>>,
        F: Fn(Self::Ok) -> Result<U, E> + Clone + Send + 'static,
        U: Send + 'static,
    {
        let n = match n.into() {
            Some(n) if n > 0 => n,
            _ => available_parallelism(),
        };

        let h = h.into().unwrap_or_else(runtime::Handle::current);

        self.broadn_and_then(n, move |val| {
            let (h, f) = (h.clone(), f.clone());

            async move { h.spawn_blocking(move || f(val)).map_err(E::from).await? }
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_item_is_computed_off_the_runtime() {
        let mut out: Vec<u8> = (1_u8..=4)
            .try_stream()
            .parallel_and_then(None, |item| Ok(item * 2))
            .try_collect()
            .await
            .expect("no errors");
        out.sort_unstable();

        assert_eq!(out, [2, 4, 6, 8]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_error_from_the_closure_is_reported() {
        let out: Result<Vec<u8>> = (1_u8..=4)
            .try_stream()
            .paralleln_and_then(None, 2, |item| {
                if item == 3 {
                    return Err(crate::err!("boom"));
                }

                Ok(item)
            })
            .try_collect()
            .await;

        assert_eq!(out.expect_err("failed").message(), "boom");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_panic_in_the_closure_becomes_a_join_error() {
        let out: Result<Vec<u8>> = (1_u8..=2)
            .try_stream()
            .parallel_and_then(None, |item| {
                assert!(item != 2, "boom");
                Ok(item)
            })
            .try_collect()
            .await;

        assert!(
            matches!(out, Err(Error::JoinError(_))),
            "a panicked worker is reported, not lost"
        );
    }
}
