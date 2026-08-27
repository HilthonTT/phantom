use futures::{Stream, StreamExt, TryStream, future::ready};

use crate::{Error, Result};

/// Drops one half of a fallible stream.
pub trait TryIgnore<'a, Item> {
    /// Yields only the successful items.
    ///
    /// In a debug build an error panics instead: a stream whose errors are
    /// worth ignoring in production is still worth noticing while developing.
    fn ignore_err(self) -> impl Stream<Item = Item> + Send + 'a;

    /// Yields only the errors.
    fn ignore_ok(self) -> impl Stream<Item = Error> + Send + 'a;
}

impl<'a, T, Item> TryIgnore<'a, Item> for T
where
    T: Stream<Item = Result<Item>> + TryStream + Send + 'a,
    Item: Send + 'a,
{
    #[cfg(debug_assertions)]
    #[inline]
    fn ignore_err(self) -> impl Stream<Item = Item> + Send + 'a {
        use super::TryExpect;

        self.expect_ok()
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn ignore_err(self) -> impl Stream<Item = Item> + Send + 'a {
        self.filter_map(|result| ready(result.ok()))
    }

    #[inline]
    fn ignore_ok(self) -> impl Stream<Item = Error> + Send + 'a {
        self.filter_map(|result| ready(result.err()))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn ignore_ok_keeps_only_the_errors() {
        let stream = vec![Ok(1_u8), Err(crate::err!("boom")), Ok(2)]
            .into_iter()
            .stream();

        let errors: Vec<String> = stream
            .ignore_ok()
            .map(|error| error.message())
            .collect()
            .await;

        assert_eq!(errors, ["boom"]);
    }

    #[tokio::test]
    async fn ignore_err_keeps_only_the_values() {
        let stream = vec![Ok(1_u8), Ok(2)].into_iter().stream();

        let items: Vec<u8> = stream.ignore_err().collect().await;

        assert_eq!(items, [1, 2]);
    }
}
