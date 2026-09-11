use super::EMPTY;

type Pair<'a> = (&'a str, &'a str);

pub trait SplitInfallible<'a> {
    fn split_once_infallible(&self, delim: &str) -> Pair<'a>;

    fn rsplit_once_infallible(&self, delim: &str) -> Pair<'a>;
}

impl<'a> SplitInfallible<'a> for &'a str {
    #[inline]
    fn rsplit_once_infallible(&self, delim: &str) -> Pair<'a> {
        self.rsplit_once(delim).unwrap_or((self, EMPTY))
    }

    #[inline]
    fn split_once_infallible(&self, delim: &str) -> Pair<'a> {
        self.split_once(delim).unwrap_or((self, EMPTY))
    }
}
