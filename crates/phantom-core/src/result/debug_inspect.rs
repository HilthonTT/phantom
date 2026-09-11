use crate::Result;

pub trait DebugInspect<T, E> {
    #[must_use]
    fn debug_inspect_err<F: FnOnce(&E)>(self, f: F) -> Self;

    #[must_use]
    fn debug_inspect<F: FnOnce(&T)>(self, f: F) -> Self;
}

#[cfg(debug_assertions)]
impl<T, E> DebugInspect<T, E> for Result<T, E> {
    #[inline]
    fn debug_inspect<F>(self, f: F) -> Self
    where
        F: FnOnce(&T),
    {
        self.inspect(f)
    }

    #[inline]
    fn debug_inspect_err<F>(self, f: F) -> Self
    where
        F: FnOnce(&E),
    {
        self.inspect_err(f)
    }
}

#[cfg(not(debug_assertions))]
impl<T, E> DebugInspect<T, E> for Result<T, E> {
    #[inline]
    fn debug_inspect<F>(self, _: F) -> Self
    where
        F: FnOnce(&T),
    {
        self
    }

    #[inline]
    fn debug_inspect_err<F>(self, _: F) -> Self
    where
        F: FnOnce(&E),
    {
        self
    }
}
