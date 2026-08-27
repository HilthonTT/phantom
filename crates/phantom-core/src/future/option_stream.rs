use futures::{Future, FutureExt, Stream, StreamExt, future::OptionFuture};

use crate::stream::IterStream;

pub trait OptionStream<T> {
    fn stream(self) -> impl Stream<Item = T> + Send;
}

impl<T, O, S, Fut> OptionStream<T> for OptionFuture<Fut>
where
    Fut: Future<Output = (O, S)> + Send,
    S: Stream<Item = T> + Send,
    O: IntoIterator<Item = T> + Send,
    <O as IntoIterator>::IntoIter: Send,
    T: Send,
{
    #[inline]
    fn stream(self) -> impl Stream<Item = T> + Send {
        self.map(|opt| opt.map(|(curr, next)| curr.into_iter().stream().chain(next)))
            .map(Option::into_iter)
            .map(IterStream::stream)
            .flatten_stream()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, executor::block_on, future::OptionFuture, future::ready, stream};

    use super::OptionStream;

    #[test]
    fn some_chains_head_then_tail() {
        let fut = OptionFuture::from(Some(ready((vec![1, 2], stream::iter(vec![3, 4])))));
        assert_eq!(block_on(fut.stream().collect::<Vec<_>>()), vec![1, 2, 3, 4]);
    }

    #[test]
    fn none_is_empty() {
        type Fut = futures::future::Ready<(Vec<u32>, stream::Iter<std::vec::IntoIter<u32>>)>;
        let fut = OptionFuture::<Fut>::from(None);
        assert!(block_on(fut.stream().collect::<Vec<_>>()).is_empty());
    }
}
