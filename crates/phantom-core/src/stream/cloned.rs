use std::clone::Clone;

use futures::{Stream, StreamExt, stream::Map};

/// [`Iterator::cloned`] for streams of references.
pub trait Cloned<'a, T, S>
where
    S: Stream<Item = &'a T>,
    T: Clone + 'a,
{
    fn cloned(self) -> Map<S, fn(&T) -> T>;
}

impl<'a, T, S> Cloned<'a, T, S> for S
where
    S: Stream<Item = &'a T>,
    T: Clone + 'a,
{
    #[inline]
    fn cloned(self) -> Map<S, fn(&T) -> T> {
        self.map(Clone::clone)
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;
    use crate::stream::IterStream;

    #[tokio::test]
    async fn clones_borrowed_items() {
        let owned = vec!["a".to_owned(), "b".to_owned()];

        let cloned: Vec<String> = owned.iter().stream().cloned().collect().await;

        assert_eq!(cloned, owned);
    }
}
