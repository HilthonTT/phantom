const QUOTE: char = '"';

pub trait Unquote<'a> {
    fn is_quoted(&self) -> bool;

    fn unquote(&self) -> Option<&'a str>;

    fn unquote_infallible(&self) -> &'a str;
}

impl<'a> Unquote<'a> for &'a str {
    #[inline]
    fn unquote(&self) -> Option<&'a str> {
        self.strip_prefix(QUOTE).and_then(|s| s.strip_suffix(QUOTE))
    }

    #[inline]
    fn is_quoted(&self) -> bool {
        self.unquote().is_some()
    }

    #[inline]
    fn unquote_infallible(&self) -> &'a str {
        self.unquote().unwrap_or(self)
    }
}
