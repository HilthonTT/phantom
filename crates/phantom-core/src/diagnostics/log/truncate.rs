use std::fmt;

pub struct TruncatedSlice<'a, T> {
    inner: &'a [T],
    max_len: usize,
}

impl<T: fmt::Debug> fmt::Debug for TruncatedSlice<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inner.len() <= self.max_len {
            write!(f, "{:?}", self.inner)
        } else {
            f.debug_list()
                .entries(&self.inner[..self.max_len])
                .entry(&"...")
                .finish()
        }
    }
}

pub fn slice_truncated<T: fmt::Debug>(
    slice: &[T],
    max_len: usize,
) -> tracing::field::DebugValue<TruncatedSlice<'_, T>> {
    tracing::field::debug(TruncatedSlice {
        inner: slice,
        max_len,
    })
}
