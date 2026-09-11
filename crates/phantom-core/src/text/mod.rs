pub mod between;
pub mod html;
pub mod quote;
pub mod split;
pub mod unquote;

pub use self::{
    between::Between, html::Escape as HtmlEscape, quote::Unquoted, split::SplitInfallible,
    unquote::Unquote,
};
use crate::{Result, exchange};

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
#[allow(clippy::unbuffered_bytes)]
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

#[must_use]
#[allow(clippy::string_slice)]
pub fn common_prefix<'a>(choice: &'a [&str]) -> &'a str {
    choice.first().map_or(EMPTY, move |best| {
        choice.iter().skip(1).fold(*best, |best, choice| {
            let len = best
                .char_indices()
                .zip(choice.chars())
                .take_while(|&((_, a), b)| a == b)
                .map(|((i, a), _)| i.saturating_add(a.len_utf8()))
                .last()
                .unwrap_or(0);

            &best[0..len]
        })
    })
}

pub fn string_from_bytes(bytes: &[u8]) -> Result<String> {
    let str: &str = str_from_bytes(bytes)?;
    Ok(str.to_owned())
}

#[inline]
pub fn str_from_bytes(bytes: &[u8]) -> Result<&str> {
    Ok(std::str::from_utf8(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::{camel_to_snake_string, common_prefix};

    #[test]
    fn common_prefix_stops_at_the_first_difference() {
        assert_eq!(common_prefix(&[]), "");
        assert_eq!(common_prefix(&["phantom"]), "phantom");
        assert_eq!(common_prefix(&["phantom", "phase", "phone"]), "ph");
        assert_eq!(common_prefix(&["phantom", "conduit"]), "");
        assert_eq!(common_prefix(&["phantom", "phantom"]), "phantom");
        assert_eq!(common_prefix(&["ph", "phantom"]), "ph");
    }

    #[test]
    fn common_prefix_cuts_on_a_character_boundary() {
        assert_eq!(common_prefix(&["日本語", "日本茶"]), "日本");
        assert_eq!(common_prefix(&["naïve", "naïvety"]), "naïve");
        assert_eq!(common_prefix(&["日本", "本日"]), "");
    }

    #[test]
    fn unquote_infallible_strips_only_a_matched_pair() {
        use super::Unquote;

        assert_eq!("\"abc\"".unquote_infallible(), "abc");
        assert_eq!("abc".unquote_infallible(), "abc");
        assert_eq!("\"abc".unquote_infallible(), "\"abc");
        assert_eq!("abc\"".unquote_infallible(), "abc\"");
        assert_eq!("\"".unquote_infallible(), "\"");
        assert_eq!("".unquote_infallible(), "");
    }

    #[test]
    fn is_quoted_agrees_with_unquote() {
        use super::Unquote;

        for input in ["\"abc\"", "abc", "\"abc", "abc\"", "\"", "", "\"\""] {
            assert_eq!(
                input.is_quoted(),
                input.unquote().is_some(),
                "is_quoted and unquote disagree about {input:?}"
            );
        }

        assert!(!"\"".is_quoted(), "a lone quote is not a quoted string");
        assert!("\"\"".is_quoted(), "two quotes are an empty quoted string");
    }

    #[test]
    fn camel_to_snake_splits_on_the_capitals() {
        assert_eq!(camel_to_snake_string("CamelCase"), "camel_case");
        assert_eq!(camel_to_snake_string("lowercase"), "lowercase");
    }
}
