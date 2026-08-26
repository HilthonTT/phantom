pub mod between;
pub mod quote;
pub mod split;
pub mod unquote;

pub use self::{between::Between, quote::Unquoted, split::SplitInfallible, unquote::Unquote};
use crate::{Result, utils::exchange};

/// The empty string, for the combinators that need a `&str` to fall back on.
pub const EMPTY: &str = "";

#[inline]
pub fn collect_stream<F>(func: F) -> Result<String>
where
    F: FnOnce(&mut dyn std::fmt::Write) -> Result<()>,
{
    let mut out = String::new();
    func(&mut out)?;
    Ok(out)
}

#[inline]
#[must_use]
pub fn camel_to_snake_string(s: &str) -> String {
    let est_len = s.chars().fold(s.len(), |est, c| {
        est.saturating_add(usize::from(c.is_ascii_uppercase()))
    });

    let mut ret = String::with_capacity(est_len);
    camel_to_snake_case(&mut ret, s.as_bytes()).expect("string-to-string stream error");
    ret
}

#[inline]
#[allow(clippy::unbuffered_bytes)] // these are allocated string utilities, not file I/O utils
pub fn camel_to_snake_case<I, O>(output: &mut O, input: I) -> Result<()>
where
    I: std::io::Read,
    O: std::fmt::Write,
{
    let mut state = false;
    input
        .bytes()
        .take_while(Result::is_ok)
        .map(Result::unwrap)
        .map(char::from)
        .try_for_each(|ch| {
            let m = ch.is_ascii_uppercase();
            let s = exchange(&mut state, !m);
            if m && s {
                output.write_char('_')?;
            }
            output.write_char(ch.to_ascii_lowercase())?;
            Result::<()>::Ok(())
        })
}

/// The longest prefix every one of `choice` starts with.
///
/// ```
/// use phantom_core::strings::common_prefix;
///
/// assert_eq!(common_prefix(&["phantom", "phase", "phone"]), "ph");
/// assert_eq!(common_prefix(&[]), "");
/// ```
#[must_use]
#[allow(clippy::string_slice)]
pub fn common_prefix<'a>(choice: &'a [&str]) -> &'a str {
    choice.first().map_or(EMPTY, move |best| {
        choice.iter().skip(1).fold(*best, |best, choice| {
            &best[0..choice
                .char_indices()
                .zip(best.char_indices())
                .take_while(|&(a, b)| a == b)
                .count()]
        })
    })
}

/// Parses the bytes into a string.
pub fn string_from_bytes(bytes: &[u8]) -> Result<String> {
    let str: &str = str_from_bytes(bytes)?;
    Ok(str.to_owned())
}

/// Parses the bytes into a string.
#[inline]
pub fn str_from_bytes(bytes: &[u8]) -> Result<&str> {
    Ok(std::str::from_utf8(bytes)?)
}
