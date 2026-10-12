//! Splits a command message into words, as a shell would for simple quoting.

use ruma::UserId;

/// Splits on whitespace; single or double quotes keep a word's spaces, and a
/// backslash takes the next character as it is.
pub(super) fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    let mut chars = line.chars();

    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                word.extend(chars.next());
                in_word = true;
            }
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }

    if quote.is_some() {
        return Err("A quote is never closed.".to_owned());
    }

    if in_word {
        words.push(word);
    }

    Ok(words)
}

/// The arguments clap parses: the invocation word (`!admin`, an escaped
/// `\!admin` or the server user's ID, with or without a trailing colon)
/// becomes the program name.
pub(super) fn argv(mut words: Vec<String>, server_user: &UserId) -> Vec<String> {
    let invoked = words.first().is_some_and(|first| {
        let first = first.trim_start_matches('\\');
        first == "!admin" || first.trim_end_matches(':') == server_user.as_str()
    });

    if invoked {
        words.remove(0);
    }

    words.insert(0, "!admin".to_owned());
    words
}
