use futures::{Stream, StreamExt, TryStream};

use crate::Result;

/// Unwraps a fallible stream, panicking on the first error.
///
/// For streams whose errors mean a bug rather than a condition the caller can
/// act on; anything recoverable belongs in [`super::TryIgnore`] or ordinary
/// `TryStream` handling.
pub trait TryExpect<'a, Item> {
    /// Unwraps each item, panicking with a generic message on an error.
    fn expect_ok(self) -> impl Stream<Item = Item> + Send + 'a;

    /// Unwraps each item, panicking with `msg` on an error.
    fn map_expect(self, msg: &'a str) -> impl Stream<Item = Item> + Send + 'a;
}

impl<'a, T, Item> TryExpect<'a, Item> for T
where
    T: Stream<Item = Result<Item>> + TryStream + Send + 'a,
    Item: 'a,
{
    #[inline]
    fn expect_ok(self) -> impl Stream<Item = Item> + Send + 'a {
        self.map_expect("stream expectation failure")
    }

    #[inline]
    fn map_expect(self, msg: &'a str) -> impl Stream<Item = Item> + Send + 'a {
        self.map(|result| result.expect(msg))
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn unwraps_every_item() {
        let items: Vec<u8> = vec![Ok(1), Ok(2)]
            .into_iter()
            .stream()
            .expect_ok()
            .collect()
            .await;

        assert_eq!(items, [1, 2]);
    }

    #[tokio::test]
    #[should_panic(expected = "the message")]
    async fn panics_on_the_first_error() {
        let stream = vec![Ok(1), Err(crate::err!("boom"))].into_iter().stream();

        let _: Vec<u8> = stream.map_expect("the message").collect().await;
    }
}
