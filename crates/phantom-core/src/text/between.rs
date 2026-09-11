type Delim<'a> = (&'a str, &'a str);

pub trait Between<'a> {
    fn between(&self, delim: Delim<'_>) -> Option<&'a str>;

    fn between_infallible(&self, delim: Delim<'_>) -> &'a str;
}

impl<'a> Between<'a> for &'a str {
    #[inline]
    fn between_infallible(&self, delim: Delim<'_>) -> &'a str {
        self.between(delim).unwrap_or(self)
    }

    #[inline]
    fn between(&self, delim: Delim<'_>) -> Option<&'a str> {
        self.split_once(delim.0)
            .and_then(|(_, b)| b.rsplit_once(delim.1))
            .map(|(a, _)| a)
    }
}
