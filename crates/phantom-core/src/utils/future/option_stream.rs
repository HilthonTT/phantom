use futures::{Future, FutureExt, Stream, StreamExt, future::OptionFuture};

use super::super::IterStream;

pub trait OptionStream<T> {
    fn stream(self) -> impl Stream<Item = T> + Send;
}
