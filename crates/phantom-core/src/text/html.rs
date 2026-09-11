use std::fmt;

pub struct Escape<'a>(pub &'a str);

#[allow(clippy::string_slice)]
impl fmt::Display for Escape<'_> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Escape(s) = *self;
        let pile_o_bits = s;
        let mut last = 0;
        for (i, ch) in s.char_indices() {
            let s = match ch {
                '>' => "&gt;",
                '<' => "&lt;",
                '&' => "&amp;",
                '\'' => "&#39;",
                '"' => "&quot;",
                _ => continue,
            };
            fmt.write_str(&pile_o_bits[last..i])?;
            fmt.write_str(s)?;
            last = i.saturating_add(1);
        }

        if last < s.len() {
            fmt.write_str(&pile_o_bits[last..])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Escape;

    #[test]
    fn escapes_every_special_character() {
        assert_eq!(
            Escape(r#"<a href='x' title="y">&</a>"#).to_string(),
            "&lt;a href=&#39;x&#39; title=&quot;y&quot;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn passes_through_plain_and_multibyte_text() {
        assert_eq!(Escape("").to_string(), "");
        assert_eq!(Escape("plain text").to_string(), "plain text");
        assert_eq!(Escape("naïve ✅ 日本").to_string(), "naïve ✅ 日本");
        assert_eq!(Escape("ünicode <b>").to_string(), "ünicode &lt;b&gt;");
    }

    #[test]
    fn keeps_the_tail_after_the_last_escape() {
        assert_eq!(Escape("a<b").to_string(), "a&lt;b");
        assert_eq!(Escape("a<").to_string(), "a&lt;");
        assert_eq!(Escape("<a").to_string(), "&lt;a");
    }
}
