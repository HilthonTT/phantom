use futures::{Stream, StreamExt, TryStream, stream};

use crate::{Error, Result};

pub trait IterStream<I: IntoIterator + Send> {
    fn stream(self) -> impl Stream<Item = I::Item> + Send;

    fn try_stream(
        self,
    ) -> impl TryStream<Ok = I::Item, Error = Error, Item = Result<I::Item>> + Send;
}

impl<I> IterStream<I> for I
where
    I: IntoIterator + Send,
    <I as IntoIterator>::IntoIter: Send,
{
    #[inline]
    fn stream(self) -> impl Stream<Item = I::Item> + Send {
        stream::iter(self)
    }

    #[inline]
    fn try_stream(
        self,
    ) -> impl TryStream<Ok = I::Item, Error = Error, Item = Result<I::Item>> + Send {
        self.stream().map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, executor::block_on};

    use super::IterStream;

    #[test]
    fn collects_in_order() {
        let items: Vec<_> = block_on(vec![1, 2, 3].stream().collect());
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn empty_source_yields_nothing() {
        let items: Vec<i32> = block_on(Vec::<i32>::new().stream().collect());
        assert!(items.is_empty());
    }

    #[test]
    fn try_stream_wraps_every_item_in_ok() {
        use futures::TryStreamExt;

        let items: Vec<_> = block_on(vec![1, 2, 3].try_stream().try_collect())
            .expect("an infallible source cannot fail");

        assert_eq!(items, vec![1, 2, 3]);
    }
}
