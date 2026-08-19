use futures::{Stream, stream};

/// Adapts any [`IntoIterator`] into a [`Stream`], so iterator sources compose
/// with the stream combinators without an explicit `stream::iter` at each site.
pub trait IterStream<I: IntoIterator + Send> {
    fn stream(self) -> impl Stream<Item = I::Item> + Send;
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
}
