use std::{fs::read_to_string, path::Path};

use crate::error;

#[must_use]
pub fn is_set(file: Option<&Path>, inline: Option<&str>) -> bool {
    file.is_some() || inline.is_some_and(|inline| !inline.is_empty())
}

#[must_use]
pub fn resolve(file: Option<&Path>, inline: Option<&str>, name: &str) -> Option<String> {
    let from_file = file.and_then(|path| {
        read_to_string(path)
            .inspect_err(|e| error!("Failed to read the {name} file {path:?}: {e}"))
            .ok()
    });

    from_file
        .as_deref()
        .map(str::trim)
        .or(inline)
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn secret_file(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("phantom-secret-{name}"));
        let mut file = std::fs::File::create(&path).expect("created");
        file.write_all(contents.as_bytes()).expect("written");

        path
    }

    #[test]
    fn nothing_configured_is_not_set() {
        assert!(!is_set(None, None));
        assert!(!is_set(None, Some("")), "an empty inline value is not one");
        assert!(is_set(None, Some("hunter2")));
    }

    #[test]
    fn a_configured_file_is_set_whatever_is_in_it() {
        assert!(is_set(Some(Path::new("/nonexistent")), None));
    }

    #[test]
    fn the_file_wins_over_the_inline_value() {
        let path = secret_file("wins", "from-file\n");

        assert_eq!(
            resolve(Some(&path), Some("inline"), "test secret").as_deref(),
            Some("from-file"),
            "and the trailing newline is trimmed"
        );
    }

    #[test]
    fn an_unreadable_file_falls_through_to_the_inline_value() {
        let missing = Path::new("/nonexistent/phantom-secret");

        assert_eq!(
            resolve(Some(missing), Some("inline"), "test secret").as_deref(),
            Some("inline")
        );
        assert_eq!(resolve(Some(missing), None, "test secret"), None);
    }

    #[test]
    fn an_empty_value_resolves_to_nothing() {
        let path = secret_file("empty", "   \n");

        assert_eq!(resolve(Some(&path), None, "test secret"), None);
        assert_eq!(resolve(None, Some(""), "test secret"), None);
        assert_eq!(resolve(None, None, "test secret"), None);
    }
}
