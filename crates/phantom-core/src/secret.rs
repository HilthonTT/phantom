//! Resolving a configured secret from a file or an inline value.
//!
//! Every secret an operator configures can be written two ways: inline in the
//! config, or as the path of a file holding it. The file is the one to prefer —
//! it keeps the secret out of a config that gets pasted into a bug report, and
//! it is what a secret manager mounts — so this resolves both the same way
//! everywhere rather than each call site deciding.
//!
//! Nothing here redacts or zeroes anything. A `String` is a `String`; treating
//! it as more than that would be a promise this cannot keep once the value has
//! been handed out.

use std::{fs::read_to_string, path::Path};

use crate::error;

/// Whether a secret is configured at all.
///
/// Answered without opening the file, so asking cannot be turned into disk
/// work by whoever is asking. A file that is configured but turns out to be
/// unreadable or blank still counts as set here; only [`resolve`] finds that
/// out.
#[must_use]
pub fn is_set(file: Option<&Path>, inline: Option<&str>) -> bool {
    file.is_some() || inline.is_some_and(|inline| !inline.is_empty())
}

/// The secret to use, from the file if there is one and the inline value
/// otherwise.
///
/// A file that was read wins, trimmed of the trailing newline an editor leaves
/// behind. A file that could *not* be read falls through to the inline value
/// and logs — an operator who configured both meant the file, but refusing to
/// start when it is briefly unreadable helps nobody. An empty result is `None`
/// rather than an empty secret, so a caller cannot accidentally accept one.
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

    /// Writes `contents` to a uniquely named file under the test temp dir.
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

    /// Asked before the file is opened, so an unreadable path still reads as
    /// configured.
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

    /// An unreadable file is a misconfiguration, not a reason to start with no
    /// secret when one was also given inline.
    #[test]
    fn an_unreadable_file_falls_through_to_the_inline_value() {
        let missing = Path::new("/nonexistent/phantom-secret");

        assert_eq!(
            resolve(Some(missing), Some("inline"), "test secret").as_deref(),
            Some("inline")
        );
        assert_eq!(resolve(Some(missing), None, "test secret"), None);
    }

    /// An empty file is a deliberate blank, and blank is not a secret.
    #[test]
    fn an_empty_value_resolves_to_nothing() {
        let path = secret_file("empty", "   \n");

        assert_eq!(resolve(Some(&path), None, "test secret"), None);
        assert_eq!(resolve(None, Some(""), "test secret"), None);
        assert_eq!(resolve(None, None, "test secret"), None);
    }
}
